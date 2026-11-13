use super::*;

fn session() -> Session {
    Session { host_id: "urn:uuid:abc".into(), ..Default::default() }
}

#[test]
fn base64url_round_trips_and_matches_the_rfc_vector() {
    // RFC 7636 appendix B: this verifier hashes to this challenge.
    let challenge = b64url(&Sha256::digest(b"dBjftJeZ4CVP-mB92K27uhbUJU1p1r_wW1gFWFOEjXk"));
    assert_eq!(challenge, "E9Melhoa2OwvFrEMTJguCHaoeK1t8URWbuGJSstw-cM");
    for data in [&b""[..], b"a", b"ab", b"abc", b"abcd", &[0xff, 0xfe, 0xfd, 0, 1]] {
        assert_eq!(b64url_decode(&b64url(data)).as_deref(), Some(data));
    }
    assert_eq!(b64url_decode("not base64!"), None);
}

#[test]
fn percent_coding_round_trips() {
    let s = "a b&c=d/é~";
    assert_eq!(percent_decode(&percent_encode(s)), s);
    assert_eq!(percent_encode("a b"), "a%20b");
    assert_eq!(percent_decode("100%"), "100%");
}

#[test]
fn host_ids_are_uuid_v4() {
    let id = new_host_id().expect("id");
    assert!(id.starts_with("urn:uuid:") && id.len() == 9 + 36, "{id}");
    assert_eq!(id.as_bytes().get(9 + 14), Some(&b'4'));
    assert_ne!(id, new_host_id().expect("id"));
}

#[test]
fn first_sign_in_registers_and_later_ones_reuse_the_client() {
    let first = build_attempt(&session(), 1455).expect("attempt");
    assert!(first.url.starts_with(AUTHORIZE_URL));
    for needle in [
        "client_id=dynamic_agent_client",
        "agent_name_hint=PdfCraft",
        "ext_agent_host_id=urn%3Auuid%3Aabc",
        "redirect_uri=http%3A%2F%2F127.0.0.1%3A1455%2Fauth%2Fcallback",
        "code_challenge_method=S256",
        "resource=https%3A%2F%2Fapi.openai.com%2Fv1",
        "chatgpt.tokens.use.direct",
    ] {
        assert!(first.url.contains(needle), "{needle} missing from {}", first.url);
    }
    let again = build_attempt(&Session { client_id: "oaiapp_1".into(), email: "me@example.com".into(), ..session() }, 5000).expect("attempt");
    assert!(again.url.contains("client_id=oaiapp_1") && !again.url.contains("agent_name_hint") && !again.url.contains("dynamic_agent_client"));
    assert!(again.url.contains("login_hint=me%40example.com"));
    assert_ne!(first.state, again.state);
}

#[test]
fn the_callback_must_carry_this_attempts_state() {
    let ok = parse_callback("GET /auth/callback?code=c%201&state=S&client_id=oaiapp_9 HTTP/1.1", "S").expect("ok");
    assert_eq!(ok, Callback { code: "c 1".into(), client_id: "oaiapp_9".into() });
    assert!(parse_callback("GET /auth/callback?code=c&state=other HTTP/1.1", "S").unwrap_err().contains("state"));
    assert!(parse_callback("GET /auth/callback?state=S&error=access_denied HTTP/1.1", "S").unwrap_err().contains("declined"));
    assert!(parse_callback("GET /auth/callback?state=S HTTP/1.1", "S").unwrap_err().contains("no code"));
    assert_eq!(parse_callback("GET /favicon.ico HTTP/1.1", "S").unwrap_err(), "not the sign-in callback");
}

#[test]
fn the_listener_hands_back_the_browsers_answer_and_ignores_strays() {
    let (listener, port) = bind_listener().expect("bind");
    let cancel = AtomicBool::new(false);
    let client = std::thread::spawn(move || {
        for path in ["/favicon.ico", "/auth/callback?code=xyz&state=S&client_id=oaiapp_2"] {
            let mut s = TcpStream::connect(("127.0.0.1", port)).expect("connect");
            write!(s, "GET {path} HTTP/1.1\r\nHost: x\r\n\r\n").expect("write");
            let mut out = String::new();
            let _ = s.read_to_string(&mut out);
        }
    });
    let cb = wait_for_callback(&listener, "S", &cancel).expect("callback");
    assert_eq!(cb.code, "xyz");
    client.join().expect("client");
}

#[test]
fn a_cancelled_sign_in_stops_waiting() {
    let (listener, _) = bind_listener().expect("bind");
    let cancel = AtomicBool::new(true);
    assert!(wait_for_callback(&listener, "S", &cancel).unwrap_err().to_string().contains("cancelled"));
}

fn jwt(claims: &Value) -> String {
    format!("{}.{}.sig", b64url(b"{}"), b64url(claims.to_string().as_bytes()))
}

#[test]
fn id_token_claims_are_checked() {
    let good = json!({ "iss": ISSUER, "aud": "oaiapp_1", "exp": 2000, "nonce": "N", "email": "me@example.com" });
    let claims = jwt_claims(&jwt(&good)).expect("claims");
    assert!(check_id_claims(&claims, "oaiapp_1", Some("N"), 1000).is_ok());
    assert!(check_id_claims(&claims, "oaiapp_1", None, 1000).is_ok());
    assert!(check_id_claims(&claims, "other", Some("N"), 1000).unwrap_err().to_string().contains("audience"));
    assert!(check_id_claims(&claims, "oaiapp_1", Some("X"), 1000).unwrap_err().to_string().contains("nonce"));
    assert!(check_id_claims(&claims, "oaiapp_1", Some("N"), 3000).unwrap_err().to_string().contains("expired"));
    let evil = json!({ "iss": "https://evil.example", "aud": "oaiapp_1", "exp": 2000 });
    assert!(check_id_claims(&evil, "oaiapp_1", None, 1000).unwrap_err().to_string().contains("issuer"));
    assert!(jwt_claims("garbage").is_none());
}

#[test]
fn tokens_are_applied_and_plan_scope_is_required() {
    let mut s = Session { client_id: "oaiapp_1".into(), ..session() };
    let id = jwt(&json!({ "iss": ISSUER, "aud": "oaiapp_1", "exp": now() + 100, "nonce": "N", "email": "me@example.com" }));
    let v = json!({ "access_token": "A", "refresh_token": "R", "id_token": id, "expires_in": 3600, "scope": "openid offline_access" });
    apply_tokens(&mut s, &v, Some("N")).expect("apply");
    assert_eq!((s.access_token.as_str(), s.refresh_token.as_str(), s.email.as_str()), ("A", "R", "me@example.com"));
    assert!(s.signed_in() && !s.plan_usage());
    s.scopes = "openid chatgpt.tokens.use.direct".into();
    assert!(s.plan_usage());
    assert!(apply_tokens(&mut s, &json!({}), None).is_err());
    assert!(apply_tokens(&mut s, &json!({ "access_token": "B", "id_token": id }), Some("wrong")).is_err());
}

#[test]
fn the_session_file_round_trips_and_sign_out_keeps_the_registration() {
    let dir = tempfile::tempdir().expect("dir");
    let path = dir.path().join("chatgpt.json");
    assert_eq!(Session::load(&path).expect("missing is empty"), Session::default());
    let s = Session {
        client_id: "oaiapp_1".into(),
        email: "me@example.com".into(),
        access_token: "A".into(),
        refresh_token: "R".into(),
        expires_at: 5,
        scopes: "x".into(),
        ..session()
    };
    s.save(&path).expect("save");
    assert_eq!(Session::load(&path).expect("load"), s);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(std::fs::metadata(&path).expect("meta").permissions().mode() & 0o777, 0o600);
    }
    assert!(!format!("{s:?}").contains('A') || !format!("{s:?}").contains("refresh"));
    sign_out(&path).expect("sign out");
    let after = Session::load(&path).expect("load");
    assert!(!after.signed_in() && after.access_token.is_empty());
    assert_eq!((after.host_id.as_str(), after.client_id.as_str()), ("urn:uuid:abc", "oaiapp_1"));
    std::fs::write(&path, "nope").expect("write");
    assert!(Session::load(&path).is_err());
}

#[test]
fn the_request_puts_system_text_in_instructions_and_never_stores() {
    let body = request_body("gpt-x", &[Message::system("be brief"), Message::user("hi"), Message::assistant("yo"), Message::user("more")]);
    assert_eq!(body["store"], false);
    assert_eq!(body["stream"], true);
    assert_eq!(body["instructions"], "be brief");
    let input = body["input"].as_array().expect("input");
    assert_eq!(input.len(), 3);
    assert!(input.iter().all(|m| m["role"] != "system"));
    assert_eq!(input[0]["content"][0]["type"], "input_text");
    assert_eq!(input[1]["content"][0]["type"], "output_text");
    assert!(body.get("temperature").is_none() && body.get("previous_response_id").is_none());
}

#[test]
fn the_stream_is_read_until_it_completes() {
    let ok = "event: response.output_text.delta\ndata: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n\n\
              data: {\"type\":\"response.output_text.delta\",\"delta\":\"lo\"}\n\n\
              data: {\"type\":\"response.completed\",\"response\":{}}\n\n";
    assert_eq!(read_stream(ok.as_bytes()).expect("ok").text, "Hello");
    let only_final =
        "data: {\"type\":\"response.completed\",\"response\":{\"output\":[{\"content\":[{\"type\":\"output_text\",\"text\":\"Hi\"}]}]}}\n";
    assert_eq!(read_stream(only_final.as_bytes()).expect("ok").text, "Hi");
    let cut = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Hel\"}\n";
    assert!(read_stream(cut.as_bytes()).unwrap_err().to_string().contains("ended before"));
    let limit =
        "data: {\"type\":\"response.failed\",\"response\":{\"error\":{\"code\":\"subscription_sharing_usage_limit_exceeded\",\"message\":\"m\"}}}\n";
    let e = read_stream(limit.as_bytes()).unwrap_err().to_string();
    assert!(e.contains("usage limit") && e.contains(USAGE_URL), "{e}");
    let partial = "data: {\"type\":\"response.output_text.delta\",\"delta\":\"Half\"}\ndata: {\"type\":\"response.incomplete\",\"response\":{\"incomplete_details\":{\"reason\":\"max_output_tokens\"}}}\n";
    assert!(read_stream(partial.as_bytes()).expect("partial").text.contains("cut short"));
}

#[test]
fn plan_errors_are_explained() {
    let e = plan_error(403, "{\"error\":{\"code\":\"subscription_sharing_user_not_eligible\",\"message\":\"x\"}}");
    assert!(e.contains("403") && e.contains("API key"), "{e}");
    assert!(plan_error(401, "{\"detail\":\"bad\"}").contains("sign in to ChatGPT again"));
    assert!(plan_error(503, "").contains("temporarily"));
}

#[test]
fn models_keep_only_listed_slugs() {
    let body = "{\"models\":[{\"slug\":\"b\",\"visibility\":\"list\"},{\"slug\":\"hidden\",\"visibility\":\"hide\"},{\"slug\":\"a\",\"visibility\":\"list\"}]}";
    assert_eq!(parse_plan_models(body).expect("ok"), vec!["a", "b"]);
    assert_eq!(parse_plan_models("{\"data\":[{\"id\":\"m\"}]}").expect("ok"), vec!["m"]);
}

#[test]
fn asking_without_a_sign_in_says_what_to_do() {
    let dir = tempfile::tempdir().expect("dir");
    let config = AiConfig {
        provider: crate::Provider::ChatGpt,
        model: "m".into(),
        chatgpt_session: Some(dir.path().join("chatgpt.json")),
        ..Default::default()
    };
    let e = crate::chat(&config, &[]).unwrap_err().to_string();
    assert!(e.contains("not signed in"), "{e}");
    let none = AiConfig { chatgpt_session: None, ..config };
    assert!(crate::list_models(&none).is_err());
}

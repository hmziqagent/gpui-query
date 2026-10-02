use crate::core::QueryError;

#[test]
fn sanitized_redacts_bearer_doubled_separator_run() {
    let clean = QueryError::response("auth failed: bearer == s3cr3t").sanitized();
    assert!(!clean.message().contains("s3cr3t"));
    assert!(clean.message().contains("[REDACTED_TOKEN]"));
}

#[test]
fn sanitized_redacts_token_doubled_separator_run() {
    let clean = QueryError::response("token == x9y8z7").sanitized();
    assert!(!clean.message().contains("x9y8z7"));
    assert!(clean.message().contains("[REDACTED_TOKEN]"));
}

#[test]
fn sanitized_redacts_bearer_colon_equals_separator_run() {
    let clean = QueryError::response("bearer := s3cr3t9").sanitized();
    assert!(!clean.message().contains("s3cr3t9"));
    assert!(clean.message().contains("[REDACTED_TOKEN]"));
}

#[test]
fn sanitized_redacts_token_equals_colon_separator_run() {
    let clean = QueryError::response("token =: leak7").sanitized();
    assert!(!clean.message().contains("leak7"));
    assert!(clean.message().contains("[REDACTED_TOKEN]"));
}

#[test]
fn sanitized_redacts_separator_run_mixed_with_whitespace() {
    let clean = QueryError::response("bearer = = val99").sanitized();
    assert!(!clean.message().contains("val99"));
    assert!(clean.message().contains("[REDACTED_TOKEN]"));
}

#[test]
fn sanitized_redacts_bearer_equals_with_surrounding_whitespace() {
    let clean = QueryError::response("auth failed: bearer = s3cr3tval").sanitized();
    assert!(!clean.message().contains("s3cr3tval"));
    assert!(clean.message().contains("[REDACTED_TOKEN]"));
}

#[test]
fn sanitized_redacts_bearer_colon_mixed_case_with_whitespace() {
    let clean = QueryError::response("BEARER : sekret9").sanitized();
    assert!(!clean.message().contains("sekret9"));
    assert!(clean.message().contains("[REDACTED_TOKEN]"));
}

#[test]
fn sanitized_redacts_bearer_tab_equals_tab_value() {
    let clean = QueryError::response("bearer\t=\tval42").sanitized();
    assert!(!clean.message().contains("val42"));
    assert!(clean.message().contains("[REDACTED_TOKEN]"));
}

#[test]
fn sanitized_redacts_token_equals_with_whitespace() {
    let clean = QueryError::response("request rejected: token = leak123").sanitized();
    assert!(!clean.message().contains("leak123"));
    assert!(clean.message().contains("[REDACTED_TOKEN]"));
}

#[test]
fn sanitized_redacts_token_colon_with_whitespace() {
    let clean = QueryError::response("Token : val456").sanitized();
    assert!(!clean.message().contains("val456"));
    assert!(clean.message().contains("[REDACTED_TOKEN]"));
}

#[test]
fn sanitized_redacts_bearer_token_after_multiple_spaces() {
    let clean = QueryError::response("Bearer  tokensecret").sanitized();
    assert!(!clean.message().contains("tokensecret"));
    assert!(clean.message().contains("[REDACTED_TOKEN]"));
}

#[test]
fn sanitized_redacts_token_after_bearer_colon() {
    let clean = QueryError::response("auth failed: bearer: abc123secret").sanitized();
    assert!(!clean.message().contains("abc123secret"));
    assert!(clean.message().contains("[REDACTED_TOKEN]"));
}

#[test]
fn sanitized_redacts_email_local_part_containing_underscore() {
    let clean = QueryError::response("login failed for alice_bob@example.com").sanitized();
    assert!(!clean.message().contains("alice"));
    assert!(clean.message().contains("[REDACTED_EMAIL]"));
}

#[test]
fn sanitized_redacts_email_with_digit_bearing_tld() {
    let clean = QueryError::response("login failed for alice@corp.c0m").sanitized();
    assert!(!clean.message().contains("alice"));
    assert!(clean.message().contains("[REDACTED_EMAIL]"));
}

#[test]
fn sanitized_redacts_email_with_two_char_digit_tld() {
    let clean = QueryError::response("no user bob@x.c0 registered").sanitized();
    assert!(!clean.message().contains("bob"));
    assert!(clean.message().contains("[REDACTED_EMAIL]"));
}

#[test]
fn sanitized_leaves_digit_first_tld_notation_untouched() {
    let clean = QueryError::response("pinned pkg@1.2.10, art@v1.2x, eve@x.c0-m").sanitized();
    assert!(clean.message().contains("pkg@1.2.10"));
    assert!(clean.message().contains("art@v1.2x"));
    assert!(clean.message().contains("eve@x.c0-m"));
    assert!(!clean.message().contains("[REDACTED_EMAIL]"));
}

#[test]
fn sanitized_redacts_email_with_digit_first_obfuscated_tld() {
    let clean = QueryError::response("login eve@x.0rg denied").sanitized();
    assert!(!clean.message().contains("eve@x.0rg"));
    assert!(clean.message().contains("[REDACTED_EMAIL]"));
}

#[test]
fn sanitized_redacts_email_with_digit_first_multi_letter_tld() {
    let clean = QueryError::response("user admin@corp.1nfo not found").sanitized();
    assert!(!clean.message().contains("admin@corp.1nfo"));
    assert!(clean.message().contains("[REDACTED_EMAIL]"));
}

#[test]
fn sanitized_redacts_email_with_trailing_fqdn_dot() {
    let clean = QueryError::response("contact alice@corp.com. now").sanitized();
    assert!(!clean.message().contains("alice@corp.com."));
    assert!(clean.message().contains("[REDACTED_EMAIL]"));
}

#[test]
fn sanitized_redacts_email_with_digit_tld_and_trailing_dot() {
    let clean = QueryError::response("leak bob@corp.c0m. here").sanitized();
    assert!(!clean.message().contains("bob@corp.c0m."));
    assert!(clean.message().contains("[REDACTED_EMAIL]"));
}

#[test]
fn sanitized_redacts_trailing_dot_hostname_but_keeps_short_digit_tld() {
    let clean = QueryError::response("seen user@2x. and host foo@bar.").sanitized();
    assert!(clean.message().contains("user@2x."));
    assert!(clean.message().contains("host [REDACTED_EMAIL]"));
    assert!(!clean.message().contains("foo@bar."));
}

#[test]
fn sanitized_redacts_dotted_local_part_with_dotless_domain() {
    let clean = QueryError::response("login failed for j.smith@intranet").sanitized();
    assert!(!clean.message().contains("j.smith@intranet"));
    assert!(clean.message().contains("[REDACTED_EMAIL]"));
}

#[test]
fn sanitized_redacts_dotless_domain_email() {
    let clean = QueryError::response("user user@intranet not found").sanitized();
    assert!(!clean.message().contains("user@intranet"));
    assert!(clean.message().contains("[REDACTED_EMAIL]"));
}

#[test]
fn sanitized_redacts_dotless_domain_email_with_trailing_dot() {
    let clean = QueryError::response("contact user@host. now").sanitized();
    assert!(!clean.message().contains("user@host"));
    assert!(clean.message().contains("[REDACTED_EMAIL]"));
}

#[test]
fn sanitized_redacts_version_like_local_at_letter_domain_token() {
    let clean = QueryError::response("build 1.0@beta failed").sanitized();
    assert!(!clean.message().contains("1.0@beta"));
    assert!(clean.message().contains("[REDACTED_EMAIL]"));
}

#[test]
fn sanitized_redacts_password_shaped_at_token_without_scheme() {
    let clean = QueryError::response("admin:P@ssw0rd! refused").sanitized();
    assert!(!clean.message().contains("P@ssw0rd"));
    assert_eq!(clean.message(), "admin:[REDACTED_EMAIL]! refused");
}

#[test]
fn sanitized_redacts_mongodb_connection_string() {
    let clean = QueryError::transport("connect mongodb://admin:secret@host/db failed").sanitized();
    assert!(clean.message().contains("[REDACTED_CONNECTION]"));
    assert!(!clean.message().contains("admin:secret"));
}

#[test]
fn sanitized_empty_message_stays_empty() {
    let clean = QueryError::response("").sanitized();
    assert_eq!(clean.message(), "");
}

#[test]
fn sanitized_redacts_email_straddling_truncation_boundary() {
    let msg = format!("{} alice@corp.com", "x".repeat(505));
    assert!(msg.len() > 512, "precondition: message must exceed the cap");
    let clean = QueryError::response(msg.as_str()).sanitized();
    assert!(
        !clean.message().contains("alice"),
        "truncation must not resurrect a partially-redacted secret"
    );
    assert!(clean.message().ends_with("...[truncated]"));
}

#[test]
fn sanitized_redacts_secret_beyond_truncation_boundary() {
    let prefix = "x".repeat(600);
    let msg = format!("{prefix}bearer s3cr3tfarpast");
    let clean = QueryError::response(msg.as_str()).sanitized();
    assert!(!clean.message().contains("s3cr3tfarpast"));
    assert!(clean.message().ends_with("...[truncated]"));
    assert!(clean.message().len() <= 512 + "...[truncated]".len());
}

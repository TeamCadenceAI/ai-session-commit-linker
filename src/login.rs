use anyhow::{Context, Result, bail};
use rand08::RngCore;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::time::{Duration, Instant};

use crate::api_client::{ApiClient, CliTokenExchangeResult};

/// Complete browser-based CLI OAuth login flow.
pub fn login_via_browser(api_base_url: &str, timeout: Duration) -> Result<CliTokenExchangeResult> {
    let nonce = generate_nonce();

    let listener =
        TcpListener::bind("127.0.0.1:0").context("failed to bind local callback port")?;
    listener
        .set_nonblocking(true)
        .context("failed to configure callback listener")?;

    let local_port = listener
        .local_addr()
        .context("failed to read local callback address")?
        .port();

    let auth_url = format!(
        "{}/auth/token?port={}&state={}",
        api_base_url.trim_end_matches('/'),
        local_port,
        nonce
    );

    open::that(&auth_url).with_context(|| {
        format!("failed to open browser. Open this URL manually to continue login: {auth_url}")
    })?;

    let deadline = Instant::now() + timeout;
    let exchange_code = wait_for_exchange_code(&listener, &nonce, deadline)?;

    let client = ApiClient::new(api_base_url);
    client
        .exchange_cli_code(&exchange_code, Duration::from_secs(10))
        .context("failed to exchange login code for CLI token")
}

fn generate_nonce() -> String {
    let mut bytes = [0u8; 16];
    rand08::thread_rng().fill_bytes(&mut bytes);
    bytes_to_hex(&bytes)
}

fn bytes_to_hex(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(HEX[(byte >> 4) as usize] as char);
        out.push(HEX[(byte & 0x0F) as usize] as char);
    }
    out
}

fn wait_for_exchange_code(
    listener: &TcpListener,
    expected_state: &str,
    deadline: Instant,
) -> Result<String> {
    loop {
        if Instant::now() >= deadline {
            bail!("login timed out waiting for browser callback");
        }

        match listener.accept() {
            Ok((mut stream, _addr)) => {
                if let Some(code) = handle_callback_request(&mut stream, expected_state)? {
                    return Ok(code);
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
            }
            Err(e) => return Err(e).context("failed while waiting for browser callback"),
        }
    }
}

fn handle_callback_request(stream: &mut TcpStream, expected_state: &str) -> Result<Option<String>> {
    stream
        .set_read_timeout(Some(Duration::from_secs(3)))
        .context("failed to set callback read timeout")?;

    let mut buffer = [0u8; 8192];
    let n = stream
        .read(&mut buffer)
        .context("failed to read callback request")?;
    if n == 0 {
        return Ok(None);
    }

    let request = String::from_utf8_lossy(&buffer[..n]);
    let mut lines = request.lines();
    let first_line = match lines.next() {
        Some(line) => line,
        None => return Ok(None),
    };

    let mut parts = first_line.split_whitespace();
    let method = parts.next().unwrap_or_default();
    let target = parts.next().unwrap_or_default();

    if method != "GET" {
        write_http_response(
            stream,
            405,
            "Method Not Allowed",
            "Only GET callbacks are supported.",
        )?;
        return Ok(None);
    }

    if !target.starts_with("/callback") {
        write_http_response(stream, 404, "Not Found", "Not a Cadence callback URL.")?;
        return Ok(None);
    }

    let url = reqwest::Url::parse(&format!("http://127.0.0.1{target}"))
        .context("failed to parse callback URL")?;

    let code = url
        .query_pairs()
        .find_map(|(k, v)| {
            if k == "code" {
                Some(v.into_owned())
            } else {
                None
            }
        })
        .unwrap_or_default();

    let returned_state = url
        .query_pairs()
        .find_map(|(k, v)| {
            if k == "state" {
                Some(v.into_owned())
            } else {
                None
            }
        })
        .unwrap_or_default();

    if code.is_empty() {
        write_http_response(
            stream,
            400,
            "Bad Request",
            "Missing exchange code in callback.",
        )?;
        return Ok(None);
    }

    if returned_state != expected_state {
        write_http_response(
            stream,
            400,
            "Bad Request",
            "State mismatch. Please retry `cadence login`.",
        )?;
        return Ok(None);
    }

    write_http_response(
        stream,
        200,
        "OK",
        "Authentication complete. You can close this tab.",
    )?;

    Ok(Some(code))
}

fn write_http_response(
    stream: &mut TcpStream,
    status_code: u16,
    status_text: &str,
    body_text: &str,
) -> Result<()> {
    let html = render_callback_html(status_code, body_text);
    let response = format!(
        "HTTP/1.1 {status_code} {status_text}\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        html.len(),
        html
    );
    stream
        .write_all(response.as_bytes())
        .context("failed to write callback response")?;
    stream
        .flush()
        .context("failed to flush callback response")?;
    Ok(())
}

fn render_callback_html(status_code: u16, body_text: &str) -> String {
    let is_success = (200..300).contains(&status_code);
    let title = if is_success {
        "Authentication Complete"
    } else {
        "Authentication Failed"
    };
    let badge = if is_success { "OK" } else { "ERR" };
    let accent = if is_success { "#10b981" } else { "#ef4444" };
    let accent_bg = if is_success {
        "rgba(16, 185, 129, 0.14)"
    } else {
        "rgba(239, 68, 68, 0.14)"
    };
    let follow_up = if is_success {
        "You can close this tab and return to your terminal."
    } else {
        "Return to your terminal for details, then run cadence login again."
    };

    let escaped_body = escape_html(body_text);
    let escaped_title = escape_html(title);
    let escaped_follow_up = escape_html(follow_up);

    format!(
        "<!doctype html>\
<html lang=\"en\">\
<head>\
<meta charset=\"utf-8\">\
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">\
<title>{escaped_title}</title>\
<style>\
* {{ box-sizing: border-box; }}\
html, body {{ height: 100%; margin: 0; }}\
body {{\
  font-family: 'Work Sans', -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;\
  color: #1a1a2e;\
  background: radial-gradient(circle at 15% 10%, rgba(79, 70, 229, 0.14) 0%, rgba(79, 70, 229, 0) 42%), #fafafa;\
}}\
.wrap {{\
  min-height: 100%;\
  display: flex;\
  align-items: center;\
  justify-content: center;\
  padding: 24px;\
}}\
.card {{\
  width: min(560px, 100%);\
  background: #ffffff;\
  border: 1px solid #e8eaed;\
  border-radius: 16px;\
  box-shadow: 0 10px 36px rgba(15, 23, 42, 0.08);\
  padding: 26px 24px;\
}}\
.brand {{\
  margin-bottom: 14px;\
}}\
.brand-logo {{\
  display: block;\
  height: 32px;\
  width: auto;\
  max-width: 140px;\
}}\
.badge {{\
  display: inline-flex;\
  align-items: center;\
  justify-content: center;\
  height: 28px;\
  padding: 0 12px;\
  border-radius: 999px;\
  font-size: 12px;\
  font-weight: 700;\
  letter-spacing: 0.08em;\
  color: {accent};\
  background: {accent_bg};\
}}\
h1 {{\
  margin: 14px 0 10px;\
  font-size: 28px;\
  line-height: 1.15;\
  color: #16213e;\
}}\
p {{\
  margin: 0;\
  line-height: 1.55;\
  color: #334155;\
}}\
.follow-up {{\
  margin-top: 14px;\
  color: #607d8b;\
}}\
@media (max-width: 480px) {{\
  .brand-logo {{\
    height: 28px;\
    max-width: 120px;\
  }}\
}}\
</style>\
</head>\
<body>\
<div class=\"wrap\">\
<main class=\"card\">\
<div class=\"brand\" aria-label=\"Cadence\">\
<svg class=\"brand-logo\" viewBox=\"0 0 128 128\" fill=\"none\" xmlns=\"http://www.w3.org/2000/svg\">\
<path d=\"M90.5996 87.5254C93.361 87.5254 95.5996 89.764 95.5996 92.5254C95.5995 94.0704 94.898 95.4511 93.7969 96.3682C93.7311 96.4291 93.6638 96.4885 93.5947 96.5459C93.4301 96.7188 93.2532 96.8794 93.0654 97.0273C92.8773 97.211 92.6751 97.3799 92.46 97.5322C92.2552 97.7287 92.0346 97.9088 91.7988 98.0684C91.4544 98.4257 91.0575 98.7311 90.6201 98.9736C90.1847 99.4117 89.6692 99.769 89.0967 100.022C88.5405 100.582 87.8532 101.011 87.084 101.259C86.3607 102.003 85.4089 102.522 84.3408 102.703C83.4381 103.622 82.1843 104.194 80.7959 104.199C79.8902 105.11 78.6362 105.675 77.25 105.675C76.8989 105.675 76.5563 105.638 76.2256 105.569C75.3215 106.469 74.0761 107.025 72.7002 107.025C71.891 107.025 71.1274 106.831 70.4512 106.49C69.5537 107.347 68.339 107.875 67 107.875C65.6173 107.875 64.3661 107.313 63.4609 106.406C62.5625 107.269 61.3437 107.8 60 107.8C57.8824 107.8 56.0737 106.483 55.3447 104.624C54.4432 105.506 53.2102 106.05 51.8496 106.05C49.0882 106.05 46.8496 103.811 46.8496 101.05C46.8496 100.524 46.9304 100.016 47.0811 99.54C46.1875 100.901 44.6497 101.8 42.9004 101.8C40.139 101.8 37.9004 99.5612 37.9004 96.7998C37.9005 94.0385 40.139 91.7998 42.9004 91.7998C45.6618 91.7998 47.9003 94.0385 47.9004 96.7998C47.9004 97.3257 47.8185 97.8325 47.668 98.3086C48.5615 96.9484 50.1005 96.0498 51.8496 96.0498C53.967 96.0498 55.7758 97.3667 56.5049 99.2256C57.4064 98.344 58.6395 97.7998 60 97.7998C61.3824 97.7998 62.633 98.3618 63.5381 99.2686C64.4366 98.4059 65.6561 97.875 67 97.875C67.8088 97.875 68.5721 98.0682 69.248 98.4092C70.1456 97.552 71.3611 97.0254 72.7002 97.0254C73.051 97.0254 73.3933 97.0612 73.7236 97.1299C74.623 96.2352 75.8611 95.6806 77.2285 95.6748C77.9465 94.9524 78.8843 94.4495 79.9326 94.2715C80.492 93.702 81.1869 93.2664 81.9648 93.0156C82.4082 92.5595 82.9377 92.1879 83.5273 91.9268C83.8594 91.5923 84.239 91.3055 84.6543 91.0752C84.8822 90.8459 85.1317 90.6379 85.4004 90.4561C85.6184 90.2299 85.8572 90.0238 86.1143 89.8418C86.2315 89.7293 86.3547 89.6232 86.4824 89.5225C86.5131 89.4925 86.5438 89.4627 86.5752 89.4336C86.7072 89.2927 86.8468 89.1591 86.9941 89.0342L87.04 88.9893C87.9448 88.0846 89.1946 87.5254 90.5752 87.5254H90.5996Z\" fill=\"#4533BB\"/>\
<path d=\"M34.1504 84.0996C36.9117 84.0996 39.1502 86.3384 39.1504 89.0996C39.1504 91.861 36.9118 94.0996 34.1504 94.0996C31.389 94.0996 29.1504 91.861 29.1504 89.0996C29.1506 86.3384 31.3891 84.0996 34.1504 84.0996Z\" fill=\"#4533BB\"/>\
<path d=\"M27.3496 72.2998C30.111 72.2998 32.3495 74.5385 32.3496 77.2998C32.3496 80.0612 30.111 82.2998 27.3496 82.2998C24.5882 82.2998 22.3496 80.0612 22.3496 77.2998C22.3497 74.5385 24.5882 72.2998 27.3496 72.2998Z\" fill=\"#4533BB\"/>\
<path d=\"M25.0752 56.6748C27.8366 56.6748 30.0751 58.9135 30.0752 61.6748C30.0752 64.4362 27.8366 66.6748 25.0752 66.6748C22.3138 66.6748 20.0752 64.4362 20.0752 61.6748C20.0753 58.9135 22.3138 56.6748 25.0752 56.6748Z\" fill=\"#4533BB\"/>\
<path d=\"M30.3496 39.2754C33.111 39.2754 35.3496 41.514 35.3496 44.2754C35.3494 47.0366 33.1109 49.2754 30.3496 49.2754C27.5883 49.2754 25.3498 47.0366 25.3496 44.2754C25.3496 41.514 27.5882 39.2754 30.3496 39.2754Z\" fill=\"#4533BB\"/>\
<path d=\"M92.5254 32.4004C95.2868 32.4004 97.5254 34.639 97.5254 37.4004C97.5252 40.1616 95.2867 42.4004 92.5254 42.4004C89.7641 42.4004 87.5256 40.1616 87.5254 37.4004C87.5254 34.639 89.764 32.4004 92.5254 32.4004Z\" fill=\"#4533BB\"/>\
<path d=\"M45.2998 24.7754C48.0612 24.7754 50.2998 27.014 50.2998 29.7754C50.2996 32.5366 48.0611 34.7754 45.2998 34.7754C42.5385 34.7754 40.3 32.5366 40.2998 29.7754C40.2998 27.014 42.5384 24.7754 45.2998 24.7754Z\" fill=\"#4533BB\"/>\
<path d=\"M68.5752 20.2754C71.3366 20.2754 73.5752 22.514 73.5752 25.2754C73.575 28.0366 71.3365 30.2754 68.5752 30.2754C65.8139 30.2754 63.5754 28.0366 63.5752 25.2754C63.5752 22.514 65.8138 20.2754 68.5752 20.2754Z\" fill=\"#4533BB\"/>\
</svg>\
</div>\
<span class=\"badge\">{badge}</span>\
<h1>{escaped_title}</h1>\
<p>{escaped_body}</p>\
<p class=\"follow-up\">{escaped_follow_up}</p>\
</main>\
</div>\
</body>\
</html>"
    )
}

fn escape_html(input: &str) -> String {
    let mut escaped = String::with_capacity(input.len());
    for c in input.chars() {
        match c {
            '&' => escaped.push_str("&amp;"),
            '<' => escaped.push_str("&lt;"),
            '>' => escaped.push_str("&gt;"),
            '"' => escaped.push_str("&quot;"),
            '\'' => escaped.push_str("&#39;"),
            _ => escaped.push(c),
        }
    }
    escaped
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nonce_is_32_hex_chars() {
        let nonce = generate_nonce();
        assert_eq!(nonce.len(), 32);
        assert!(nonce.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn hex_encoder_round_trip_length() {
        let bytes = [0xde, 0xad, 0xbe, 0xef];
        let hex = bytes_to_hex(&bytes);
        assert_eq!(hex, "deadbeef");
    }

    #[test]
    fn callback_html_success_variant_is_styled() {
        let html = render_callback_html(200, "Authentication complete. You can close this tab.");
        assert!(html.contains("Authentication Complete"));
        assert!(html.contains(">OK<"));
        assert!(html.contains("#10b981"));
        assert!(html.contains("Work Sans"));
        assert!(html.contains("class=\"brand-logo\""));
        assert!(html.contains("fill=\"#4533BB\""));
    }

    #[test]
    fn callback_html_error_variant_is_styled() {
        let html = render_callback_html(400, "State mismatch. Please retry cadence login.");
        assert!(html.contains("Authentication Failed"));
        assert!(html.contains(">ERR<"));
        assert!(html.contains("#ef4444"));
        assert!(html.contains("run cadence login again"));
    }

    #[test]
    fn callback_html_escapes_message_content() {
        let html = render_callback_html(400, "<script>alert('xss')</script>");
        assert!(html.contains("&lt;script&gt;alert(&#39;xss&#39;)&lt;/script&gt;"));
        assert!(!html.contains("<script>alert('xss')</script>"));
    }
}

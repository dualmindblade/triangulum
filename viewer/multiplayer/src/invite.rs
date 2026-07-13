use std::fmt;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Invite {
    pub websocket_url: String,
    pub token: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct InviteError(pub String);

impl fmt::Display for InviteError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}
impl std::error::Error for InviteError {}

pub fn parse_invite(input: &str) -> Result<Invite, InviteError> {
    let input = input.trim();
    if let Some(rest) = input.strip_prefix("triangulum://") {
        let (authority_path, token) = rest
            .rsplit_once('#')
            .ok_or_else(|| InviteError("triangulum invite is missing #token".into()))?;
        let authority = authority_path.trim_end_matches('/');
        if authority.is_empty() || !authority.contains(':') {
            return Err(InviteError("triangulum invite needs host:port".into()));
        }
        if token.is_empty() {
            return Err(InviteError("invite token is empty".into()));
        }
        return Ok(Invite {
            websocket_url: format!("ws://{authority}/?token={token}"),
            token: token.to_string(),
        });
    }
    if input.starts_with("wss://") {
        return Err(InviteError(
            "wss:// is not enabled in MP1; use the server's printed ws:// invite".into(),
        ));
    }
    if input.starts_with("ws://") {
        let without_fragment = input.split('#').next().unwrap_or(input);
        let token = query_value(without_fragment, "token")
            .or_else(|| input.split_once('#').map(|(_, v)| v))
            .filter(|v| !v.is_empty())
            .ok_or_else(|| InviteError("WebSocket invite is missing ?token=...".into()))?;
        return Ok(Invite {
            websocket_url: without_fragment.to_string(),
            token: token.to_string(),
        });
    }
    Err(InviteError(
        "invite must start with triangulum:// or ws://".into(),
    ))
}

fn query_value<'a>(url: &'a str, key: &str) -> Option<&'a str> {
    let (_, query) = url.split_once('?')?;
    query.split('&').find_map(|part| {
        let (k, v) = part.split_once('=')?;
        (k == key).then_some(v)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_both_printed_invite_forms() {
        let custom = parse_invite("triangulum://127.0.0.1:7777/#abc123").unwrap();
        assert_eq!(custom.websocket_url, "ws://127.0.0.1:7777/?token=abc123");
        assert_eq!(parse_invite(&custom.websocket_url).unwrap(), custom);
    }
}

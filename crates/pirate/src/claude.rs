//! What Claude Code wrote down, and what Anthropic answers.
//!
//! Two JSON shapes and nothing else. The host hands over bytes — it has no
//! idea what a credential is or what a limit is — so reading them is the
//! plugin's work, and getting them wrong is the plugin's problem rather than
//! the terminal's.
//!
//! Both shapes are read with `#[serde(default)]` on everything optional and no
//! `deny_unknown_fields` anywhere: this is somebody else's format, it will
//! grow fields, and a plugin that refused the whole answer because one of them
//! was new would be a chip that goes blank on a Tuesday.

use serde::Deserialize;

/// Where Claude Code keeps the session it is signed in with.
///
/// The path is also the capability: it is what the manifest asks for, what the
/// permission dialog prints, and what the host checks a read against — the
/// same string in all four places, which is why it is a constant rather than
/// something assembled.
pub const CREDENTIALS_PATH: &str = "~/.claude/.credentials.json";

/// The endpoint that knows how much of the limits is gone.
pub const USAGE_URL: &str = "https://api.anthropic.com/api/oauth/usage";

/// The host that URL names, which is the other half of what is asked for.
pub const USAGE_HOST: &str = "api.anthropic.com";

/// The beta header the OAuth-authenticated endpoints require.
pub const OAUTH_BETA: &str = "oauth-2025-04-20";

/// Refuse to reuse a token that would expire mid-request.
const EXPIRY_SKEW_MILLIS: i64 = 60_000;

/// A session read out of the credentials file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// What goes in the `authorization` header.
    pub token: String,
    /// When it stops working, if the file says.
    pub expires_at: Option<i64>,
}

impl Session {
    /// Whether this can be used again instead of read from disk again.
    ///
    /// With a minute of margin, because a token that expires while the request
    /// is in flight is a 401 that looks like a signed-out session.
    pub fn is_usable(&self, now: i64) -> bool {
        self.expires_at
            .is_none_or(|expires_at| expires_at > now + EXPIRY_SKEW_MILLIS)
    }
}

/// The part of Claude Code's credential blob this needs.
#[derive(Debug, Deserialize)]
struct StoredCredentials {
    #[serde(rename = "claudeAiOauth")]
    claude_ai_oauth: StoredOauth,
}

#[derive(Debug, Deserialize)]
struct StoredOauth {
    #[serde(rename = "accessToken")]
    access_token: String,
    /// Milliseconds since the epoch.
    #[serde(default, rename = "expiresAt")]
    expires_at: Option<i64>,
}

/// The session in a credentials file, or `None` if there is not one in it.
pub fn read_session(bytes: &[u8]) -> Option<Session> {
    let stored: StoredCredentials = serde_json::from_slice(bytes).ok()?;
    let token = stored.claude_ai_oauth.access_token;
    (!token.is_empty()).then_some(Session {
        token,
        expires_at: stored.claude_ai_oauth.expires_at,
    })
}

/// One limit, and when its window rolls over.
#[derive(Copy, Clone, Debug, Default, PartialEq)]
pub struct Limit {
    /// How much of it is gone, 0–100. Already a percentage on the wire, not a
    /// ratio.
    pub percent: f32,
    /// When it rolls over, in milliseconds since the epoch, when the plan
    /// reports one.
    pub resets_at: Option<i64>,
}

/// A point-in-time view of what Claude says is left.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Reading {
    /// The rolling five-hour window every plan has.
    pub session: Limit,
    /// The weekly window, when the plan has one.
    pub weekly: Option<Limit>,
    /// Pay-as-you-go past the plan, in whole currency units, when it is on.
    pub extra: Option<(f64, f64)>,
}

#[derive(Debug, Deserialize)]
struct UsageResponse {
    #[serde(default)]
    five_hour: Option<Bucket>,
    #[serde(default)]
    seven_day: Option<Bucket>,
    #[serde(default)]
    extra_usage: Option<ExtraUsage>,
}

#[derive(Debug, Deserialize)]
struct Bucket {
    #[serde(default)]
    utilization: f32,
    #[serde(default)]
    resets_at: Option<String>,
}

#[derive(Debug, Deserialize)]
struct ExtraUsage {
    #[serde(default)]
    is_enabled: bool,
    #[serde(default)]
    monthly_limit: Option<f64>,
    #[serde(default)]
    used_credits: Option<f64>,
}

impl Bucket {
    fn into_limit(self) -> Limit {
        Limit {
            percent: self.utilization,
            resets_at: self
                .resets_at
                .as_deref()
                .and_then(crate::time::parse_rfc3339),
        }
    }
}

/// What the endpoint said, or `None` if it was not that.
///
/// A missing `five_hour` is a session at nothing rather than a failure: the
/// endpoint answers that way for an account that has not spent anything, and
/// "no reading" and "nothing used" are different things to draw.
pub fn read_usage(bytes: &[u8]) -> Option<Reading> {
    let response: UsageResponse = serde_json::from_slice(bytes).ok()?;

    let extra = response.extra_usage.and_then(|extra| {
        let (used, limit) = (extra.used_credits?, extra.monthly_limit?);
        // Credits are minor units — cents — and a panel says dollars.
        (extra.is_enabled && limit > 0.).then_some((used / 100., limit / 100.))
    });

    Some(Reading {
        session: response
            .five_hour
            .map(Bucket::into_limit)
            .unwrap_or_default(),
        weekly: response.seven_day.map(Bucket::into_limit),
        extra,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_session_is_read_out_of_the_file_claude_code_writes() {
        let file = br#"{"claudeAiOauth":{"accessToken":"sk-ant-oat01-x","expiresAt":1788546600000,
                        "refreshToken":"whatever","scopes":["user:inference"]}}"#;

        assert_eq!(
            read_session(file),
            Some(Session {
                token: "sk-ant-oat01-x".into(),
                expires_at: Some(1_788_546_600_000),
            })
        );
    }

    #[test]
    fn a_file_with_no_session_in_it_is_not_an_error_to_report() {
        // Every one of these is "Claude Code has not signed in here", which
        // the chip says in one word rather than as a parse failure.
        for file in [
            &b"{}"[..],
            br#"{"claudeAiOauth":{"accessToken":""}}"#,
            br#"{"claudeAiOauth":{}}"#,
            b"not json at all",
        ] {
            assert_eq!(read_session(file), None);
        }
    }

    #[test]
    fn a_token_with_no_expiry_is_used_until_it_stops_working() {
        let forever = Session {
            token: "x".into(),
            expires_at: None,
        };

        assert!(forever.is_usable(i64::MAX - 1));
    }

    #[test]
    fn a_token_about_to_expire_is_read_again_rather_than_sent() {
        let session = Session {
            token: "x".into(),
            expires_at: Some(1_000_000),
        };

        assert!(session.is_usable(1_000_000 - 60_001));
        // Inside the minute of margin: still valid, and not worth sending.
        assert!(!session.is_usable(1_000_000 - 60_000));
        assert!(!session.is_usable(1_000_001));
    }

    #[test]
    fn a_reading_is_read_out_of_what_the_endpoint_answers() {
        let answer = br#"{
            "five_hour": {"utilization": 47.5, "resets_at": "2026-09-04T18:30:00Z"},
            "seven_day": {"utilization": 62.0, "resets_at": "2026-09-08T00:00:00Z"},
            "extra_usage": {"is_enabled": true, "monthly_limit": 5000.0, "used_credits": 1234.0}
        }"#;

        let reading = read_usage(answer).expect("that is a reading");

        assert_eq!(reading.session.percent, 47.5);
        assert_eq!(reading.session.resets_at, Some(1_788_546_600_000));
        assert_eq!(reading.weekly.expect("a weekly limit").percent, 62.0);
        assert_eq!(reading.extra, Some((12.34, 50.)));
    }

    #[test]
    fn an_account_that_has_spent_nothing_reads_as_nothing_spent() {
        // Not as "no reading": the two are drawn differently, and the
        // endpoint answers this way for a fresh account.
        let reading = read_usage(b"{}").expect("an empty answer is still an answer");

        assert_eq!(reading.session, Limit::default());
        assert_eq!(reading.weekly, None);
        assert_eq!(reading.extra, None);
    }

    #[test]
    fn extra_usage_nobody_turned_on_is_not_drawn() {
        let off =
            br#"{"extra_usage":{"is_enabled":false,"monthly_limit":5000.0,"used_credits":10.0}}"#;
        let unset =
            br#"{"extra_usage":{"is_enabled":true,"monthly_limit":0.0,"used_credits":0.0}}"#;

        assert_eq!(read_usage(off).expect("a reading").extra, None);
        assert_eq!(read_usage(unset).expect("a reading").extra, None);
    }

    #[test]
    fn an_answer_that_is_not_json_is_not_a_reading() {
        assert_eq!(read_usage(b"<html>a proxy said no</html>"), None);
    }
}

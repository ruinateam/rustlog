//! Chat commands the bot answers: `!rustlog <command> <arguments>`.

use crate::state::OptOutScope;

pub const PREFIX: &str = "!rustlog ";

#[derive(Debug, PartialEq, Eq)]
pub enum Command<'a> {
    /// Admins only: start logging channels, by login.
    Join(Vec<&'a str>),
    /// Admins only: stop logging channels, by login.
    Leave(Vec<&'a str>),
    /// Opt out of logging, or back in.
    OptOut(OptOutChange<'a>),
}

#[derive(Debug, PartialEq, Eq)]
pub struct OptOutChange<'a> {
    pub scope: OptOutScope,
    pub opted_out: bool,
    /// An opt-out code, which acts for the sender (or for the channel, when
    /// its broadcaster sends it in its own chat), or for admins a login.
    pub argument: &'a str,
}

/// The command in a chat message, if it is one.
pub fn parse(message: &str) -> Option<Command<'_>> {
    let mut words = message.strip_prefix(PREFIX)?.split_whitespace();
    let action = words.next()?;
    let arguments: Vec<&str> = words.collect();

    let opt_out = |scope, opted_out| {
        arguments.first().map(|argument| {
            Command::OptOut(OptOutChange {
                scope,
                opted_out,
                argument,
            })
        })
    };
    match action {
        "join" => Some(Command::Join(arguments)),
        "leave" | "part" => Some(Command::Leave(arguments)),
        "optout" => opt_out(OptOutScope::User, true),
        "optin" => opt_out(OptOutScope::User, false),
        "optout-channel" => opt_out(OptOutScope::Channel, true),
        "optin-channel" => opt_out(OptOutScope::Channel, false),
        _ => None,
    }
}

/// Who a valid opt-out code acts for: the sender, or for a channel scope
/// the channel of the chat when the sender is its broadcaster.
pub fn code_subject<'a>(
    scope: OptOutScope,
    sender_id: &'a str,
    channel_id: &'a str,
) -> Result<&'a str, &'static str> {
    match scope {
        OptOutScope::User => Ok(sender_id),
        OptOutScope::Channel if sender_id == channel_id => Ok(channel_id),
        OptOutScope::Channel => Err("only the broadcaster can opt a channel out, in its own chat"),
    }
}

#[cfg(test)]
mod tests {
    use super::{Command, OptOutChange, code_subject, parse};
    use crate::state::OptOutScope;

    #[test]
    fn parses_opt_out_commands() {
        let change = |scope, opted_out| {
            Some(Command::OptOut(OptOutChange {
                scope,
                opted_out,
                argument: "a1B2c",
            }))
        };
        assert_eq!(
            parse("!rustlog optout a1B2c"),
            change(OptOutScope::User, true)
        );
        assert_eq!(
            parse("!rustlog optin a1B2c"),
            change(OptOutScope::User, false)
        );
        assert_eq!(
            parse("!rustlog optout-channel  a1B2c"),
            change(OptOutScope::Channel, true)
        );
        assert_eq!(
            parse("!rustlog optin-channel a1B2c extra"),
            change(OptOutScope::Channel, false)
        );
    }

    #[test]
    fn parses_channel_commands() {
        assert_eq!(
            parse("!rustlog join a b"),
            Some(Command::Join(vec!["a", "b"]))
        );
        assert_eq!(parse("!rustlog part a"), Some(Command::Leave(vec!["a"])));
    }

    #[test]
    fn ignores_other_messages() {
        assert_eq!(parse("hello"), None);
        assert_eq!(parse("!rustlog"), None);
        assert_eq!(parse("!rustlog optout"), None);
        assert_eq!(parse("!rustlog dance"), None);
        assert_eq!(parse(" !rustlog optout abc"), None);
    }

    #[test]
    fn channel_codes_need_the_broadcaster() {
        assert_eq!(code_subject(OptOutScope::User, "22", "11"), Ok("22"));
        assert_eq!(code_subject(OptOutScope::Channel, "11", "11"), Ok("11"));
        assert!(code_subject(OptOutScope::Channel, "22", "11").is_err());
    }
}

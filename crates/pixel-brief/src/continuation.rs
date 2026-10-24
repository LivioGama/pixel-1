// SPDX-FileCopyrightText: The RTO contributors
// SPDX-License-Identifier: MIT

//! Which prompts are continuations rather than tasks: acknowledgements,
//! greetings and the envelopes a harness submits on the user's behalf. The
//! per-prompt evidence brief skips them.

/// Openings of a turn the harness wrote, not the human. The tags left open
/// (`<local-command`, and Codex's two) also match one that carries an
/// attribute.
pub const ORCHESTRATOR_PREFIXES: &[&str] = &[
    "<system-reminder>",
    "<task-notification>",
    "<command-name>",
    "<command-message>",
    "<local-command",
    "<bash-input>",
    "<realtime_delegation",
    "<send_user_message_question_reply",
    "[SYSTEM NOTIFICATION",
    "Caveat: The messages below",
    "[Request interrupted",
];

/// The headings Codex's IDE extension writes above what the user typed,
/// after its own listing of the attached files (`# Files mentioned by the
/// user:`, one `## <name>: <path>` per image).
const CODEX_REQUEST_HEADINGS: [&str; 2] = ["## My request:", "## My request for Codex:"];

/// The part of `prompt` the user typed: what follows the first line that is a
/// Codex request heading, the whole prompt when there is none. `## My
/// request:\ngo next` is the acknowledgement `go next`, not a prompt about a
/// request (#1032).
pub(crate) fn user_request(prompt: &str) -> &str {
    let mut offset = 0;
    for line in prompt.split_inclusive('\n') {
        offset += line.len();
        if CODEX_REQUEST_HEADINGS.contains(&line.trim()) {
            return &prompt[offset..];
        }
    }
    prompt
}

const SYSTEM_REMINDER_OPEN: &str = "<system-reminder>";
const SYSTEM_REMINDER_CLOSE: &str = "</system-reminder>";

/// A prompt the harness submitted on the user's behalf: a background-task
/// `<task-notification>`, a slash-command or local-command wrapper, one of
/// Codex's `<realtime_delegation>` and `<send_user_message_question_reply>`
/// envelopes, or nothing but `<system-reminder>` blocks. It must neither
/// rewrite the task packet nor open a boundary. Only the opening counts (after
/// any leading reminders), so a human prompt that quotes an envelope mid-text
/// is still a prompt.
pub(crate) fn is_harness_envelope(prompt: &str) -> bool {
    let mut rest = prompt.trim_start();
    while let Some(after) = rest.strip_prefix(SYSTEM_REMINDER_OPEN) {
        let Some(end) = after.find(SYSTEM_REMINDER_CLOSE) else {
            return true;
        };
        rest = after[end + SYSTEM_REMINDER_CLOSE.len()..].trim_start();
    }
    rest.is_empty()
        || ORCHESTRATOR_PREFIXES
            .iter()
            .any(|prefix| rest.starts_with(prefix))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TASK_NOTIFICATION: &str = "<task-notification>\n<task-id>b1f0c2</task-id>\n<status>completed</status>\n<summary>Agent \"fix auth\" completed</summary>\n</task-notification>";

    #[test]
    fn harness_envelopes_are_continuations() {
        for prompt in [
            TASK_NOTIFICATION,
            "  <task-notification><task-id>x</task-id></task-notification>",
            "<system-reminder>ctx</system-reminder>",
            "<system-reminder>a</system-reminder>\n<system-reminder>b</system-reminder>\n",
            "<system-reminder>unterminated",
            "<system-reminder>ctx</system-reminder><task-notification>done</task-notification>",
            "<command-name>/clear</command-name>",
            "<command-message>review</command-message>",
            "<local-command-caveat>Caveat: generated locally</local-command-caveat>",
            "<local-command-stdout>ok</local-command-stdout>",
            "<realtime_delegation><input>where is x</input></realtime_delegation>",
            "<send_user_message_question_reply>[]</send_user_message_question_reply>",
        ] {
            assert!(is_harness_envelope(prompt), "{prompt}");
        }
    }

    /// Codex's two envelopes, with an identifier in the body that would make
    /// either one a code-shaped prompt.
    fn codex_envelopes() -> [String; 2] {
        [
            [
                "<realtime_delegation>",
                "  <input>where is fetchUser</input>",
                "</realtime_delegation>",
            ]
            .join("\n"),
            [
                "<send_user_message_question_reply>",
                "[{\"question\":\"where is fetchUser\",\"answer\":\"src/api.rs\"}]",
                "</send_user_message_question_reply>",
            ]
            .join("\n"),
        ]
    }

    #[test]
    fn codex_envelopes_are_harness_envelopes_whatever_precedes_them() {
        for envelope in codex_envelopes() {
            for prompt in [
                envelope.clone(),
                format!("  \n{envelope}"),
                format!("<system-reminder>ctx</system-reminder>\n{envelope}"),
                format!(
                    "<system-reminder>a</system-reminder>\n<system-reminder>b</system-reminder>{envelope}"
                ),
            ] {
                assert!(is_harness_envelope(&prompt), "{prompt}");
            }
        }
        // A tag that carries an attribute is the same envelope.
        for prompt in [
            "<realtime_delegation id=\"rt-1\">x</realtime_delegation>",
            "<send_user_message_question_reply id=\"q-1\">[]</send_user_message_question_reply>",
        ] {
            assert!(is_harness_envelope(prompt), "{prompt}");
        }
    }

    #[test]
    fn prompts_that_quote_an_envelope_are_still_prompts() {
        for prompt in [
            "fix the hook: a <task-notification> prompt overwrites the task",
            "why does task-notification reach the packet",
            "<system-reminder>ctx</system-reminder>\nfix the login bug",
            "<system-reminder>ctx</system-reminder>fix auth",
            "implement <command-name> parsing",
            "fix the hook: a <realtime_delegation> prompt starts a brief",
            "why does send_user_message_question_reply reach the brief",
            "<system-reminder>ctx</system-reminder>\nwhere is fetchUser, a <realtime_delegation> says",
        ] {
            assert!(!is_harness_envelope(prompt), "{prompt}");
        }
    }

    #[test]
    fn user_request_should_be_the_text_under_the_first_codex_request_heading() {
        assert_eq!(user_request("## My request:\ngo next"), "go next");
        assert_eq!(
            user_request(
                "# Files mentioned by the user:\n## a.png: /tmp/a.png\n## My request for Codex:\nfix auth\n"
            ),
            "fix auth\n"
        );
        // Only the first heading counts: the user's text may quote one.
        assert_eq!(
            user_request("## My request:\nwhy does `## My request:`\nreach the brief"),
            "why does `## My request:`\nreach the brief"
        );
        for prompt in [
            "fix auth",
            "see ## My request: inline",
            "## My requests:\nx",
            "",
        ] {
            assert_eq!(user_request(prompt), prompt, "{prompt:?}");
        }
    }
}

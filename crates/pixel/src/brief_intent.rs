// SPDX-FileCopyrightText: The Pixel contributors
// SPDX-License-Identifier: MIT

//! The brief's optional task-intent judgment, made by the configured
//! `pixel classify` engine in this CLI process.

use std::io::{Read, Write};
use std::process::{Command, Stdio};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

const OUTPUT_CAP: u64 = 256 * 1024;

fn request(prompt: &str) -> Value {
    let mut labels = crate::task_intent::INTENTS
        .iter()
        .map(|intent| intent.label)
        .collect::<Vec<_>>();
    labels.push("none");
    let mut criteria = serde_json::Map::new();
    for intent in crate::task_intent::INTENTS {
        criteria.insert(intent.label.to_owned(), json!(intent.criterion));
    }
    criteria.insert(
        "none".to_owned(),
        json!("not a coding task: chat, a git or release request, prose, or anything the other labels do not cover"),
    );
    json!({
        "text": prompt,
        "context": crate::task_intent::INTENT_CONTEXT,
        "labels": labels,
        "criteria": criteria,
    })
}

fn typed_prompt(prompt: &str) -> Option<String> {
    pixel_brief::prompt::classifier_text(prompt)
}

pub(crate) fn deadline_unix_ms(deadline: SystemTime) -> Option<u64> {
    u64::try_from(deadline.duration_since(UNIX_EPOCH).ok()?.as_millis()).ok()
}

/// Ask the configured classifier once within the caller's remaining brief
/// budget. Any setup, process, timeout, or response error is fail-open.
#[cfg(not(test))]
pub(crate) fn classify(prompt: &str, deadline: Instant) -> Option<Value> {
    if Instant::now() >= deadline {
        return None;
    }
    if !crate::classify_config::classify_enabled().ok()? {
        return None;
    }
    let prompt = typed_prompt(prompt)?;
    if Instant::now() >= deadline {
        return None;
    }
    let executable = std::env::current_exe().ok()?;
    let mut command = Command::new(executable);
    command.args(["--metrics", "off", "classify", "--jsonl"]);
    classify_with(&mut command, &prompt, deadline)
}

/// Hook and command tests never read a developer's classify configuration or
/// start a provider process. `classify_with` remains covered by fake children.
#[cfg(test)]
pub(crate) fn classify(_prompt: &str, _deadline: Instant) -> Option<Value> {
    None
}

fn classify_with(command: &mut Command, prompt: &str, deadline: Instant) -> Option<Value> {
    let input = serde_json::to_vec(&request(prompt)).ok()?;
    if Instant::now() >= deadline {
        return None;
    }

    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null());
    let mut child = command.spawn().ok()?;
    let Some(stdout) = child.stdout.take() else {
        let _ = child.kill();
        let _ = child.wait();
        return None;
    };
    let (output_sender, output_receiver) = mpsc::sync_channel(1);
    let reader = thread::spawn(move || {
        let mut output = Vec::new();
        let result = stdout
            .take(OUTPUT_CAP + 1)
            .read_to_end(&mut output)
            .map(|_| output);
        let _ = output_sender.send(result);
    });
    let Some(mut stdin) = child.stdin.take() else {
        let _ = child.kill();
        let _ = child.wait();
        join_if_ready(reader, output_receiver, Duration::from_millis(25));
        return None;
    };
    let (writer_sender, writer_receiver) = mpsc::sync_channel(1);
    let writer = thread::spawn(move || {
        let result = stdin
            .write_all(&input)
            .and_then(|()| stdin.write_all(b"\n"));
        let _ = writer_sender.send(result.is_ok());
    });

    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    join_if_ready(writer, writer_receiver, Duration::from_millis(25));
                    join_if_ready(reader, output_receiver, Duration::from_millis(25));
                    return None;
                }
                let remaining = deadline.saturating_duration_since(Instant::now());
                if !join_if_ready(writer, writer_receiver, remaining)? {
                    join_if_ready(reader, output_receiver, Duration::from_millis(25));
                    return None;
                }
                let output = join_if_ready(reader, output_receiver, remaining)?.ok()?;
                if output.len() as u64 > OUTPUT_CAP {
                    return None;
                }
                let document: Value = serde_json::from_slice(&output).ok()?;
                return (document["ok"] == true
                    && document["predicted"].is_string()
                    && document["probs"].is_object()
                    && document["snapshot"].is_object())
                .then_some(document);
            }
            Ok(None) if Instant::now() < deadline => {
                thread::sleep(Duration::from_millis(5));
            }
            Ok(None) | Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                join_if_ready(writer, writer_receiver, Duration::from_millis(25));
                join_if_ready(reader, output_receiver, Duration::from_millis(25));
                return None;
            }
        }
    }
}

fn join_if_ready<T: Send + 'static>(
    thread: thread::JoinHandle<()>,
    receiver: Receiver<T>,
    timeout: Duration,
) -> Option<T> {
    match receiver.recv_timeout(timeout) {
        Ok(value) => {
            let _ = thread.join();
            Some(value)
        }
        Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => None,
    }
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    const RESPONSE: &str = r#"{"ok":true,"predicted":"investigate","probs":{"investigate":0.7,"none":0.3},"snapshot":{"provider":"test","model":"fake"}}"#;

    #[test]
    fn request_uses_the_existing_task_intents_and_adds_none_without_rewriting_text() {
        let prompt = "Explain how the list of posts works";
        let value = request(prompt);
        let labels = value["labels"].as_array().unwrap();
        assert_eq!(labels.len(), crate::task_intent::INTENTS.len() + 1);
        assert_eq!(labels.last().unwrap(), "none");
        assert_eq!(value["text"], prompt);
        assert_eq!(value["context"], crate::task_intent::INTENT_CONTEXT);
        assert!(
            value["criteria"]["none"]
                .as_str()
                .unwrap()
                .contains("not a coding task")
        );
    }

    #[test]
    fn typed_task_survives_a_large_wrapper_before_the_request_heading() {
        let wrapper = format!(
            "## Files in context\n{}\n## My request:\nExplain how the list of posts works",
            "x".repeat(OUTPUT_CAP as usize * 8)
        );
        assert_eq!(
            typed_prompt(&wrapper).as_deref(),
            Some("Explain how the list of posts works")
        );
    }

    #[test]
    fn deadline_conversion_keeps_epoch_milliseconds() {
        let deadline = UNIX_EPOCH + Duration::from_millis(1_800_000_000_123);
        assert_eq!(deadline_unix_ms(deadline), Some(1_800_000_000_123));
    }

    #[test]
    fn configured_jsonl_child_result_is_returned_as_the_raw_document() {
        let mut command = Command::new("sh");
        command.args([
            "-c",
            &format!("cat >/dev/null; printf '%s\\n' '{RESPONSE}'"),
        ]);
        let value = classify_with(
            &mut command,
            "explain the post list",
            Instant::now() + Duration::from_secs(2),
        )
        .unwrap();
        assert_eq!(value["predicted"], "investigate");
        assert_eq!(value["snapshot"]["provider"], "test");
    }

    #[test]
    fn child_failure_and_timeout_fail_open() {
        let mut failure = Command::new("sh");
        failure.args(["-c", "cat >/dev/null; exit 1"]);
        assert!(
            classify_with(
                &mut failure,
                "explain the post list",
                Instant::now() + Duration::from_secs(2)
            )
            .is_none()
        );

        let mut sleeper = Command::new("sh");
        sleeper.args(["-c", "cat >/dev/null; exec sleep 5"]);
        assert!(
            classify_with(
                &mut sleeper,
                "explain the post list",
                Instant::now() + Duration::from_millis(25)
            )
            .is_none()
        );
    }

    #[test]
    fn a_child_that_does_not_read_stdin_is_killed_within_the_deadline() {
        let mut child = Command::new("sh");
        child.args(["-c", "exec sleep 5"]);
        let prompt = "x".repeat(4 * 1024 * 1024);
        let started = Instant::now();
        assert!(classify_with(&mut child, &prompt, started + Duration::from_millis(100)).is_none());
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}

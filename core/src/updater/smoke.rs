//! Runs smoke-test.py: PDF, Word, PowerPoint, Excel, Playwright and GPU Whisper. Run before and after a Python
//! update; a test that passed before and fails after means the update broke something (av 19, 3 Nov).

use std::time::Duration;

use super::storage;
use crate::proc::{self, Cancel, minutes};

#[derive(Clone, Debug, Default)]
pub struct SmokeResult {
    pub ran: bool,
    pub passed: Vec<String>,
    pub failed: Vec<String>,
    pub raw: String,
}

/// The test script, carried inside the exe and written next to the logs when needed.
pub const SCRIPT: &str = include_str!("../../assets/smoke-test.py");

const SPEECH_SCRIPT: &str = r#"
Add-Type -AssemblyName System.Speech
$t = New-Object System.Speech.Synthesis.SpeechSynthesizer
$t.SetOutputToWaveFile($env:SU_WAV)
$t.Speak('The quick brown fox jumps over the lazy dog.')
$t.Dispose()
"#;

pub fn run(python: &str, cancel: Option<&Cancel>) -> SmokeResult {
    let script = storage::root().join("smoke-test.py");
    if std::fs::write(&script, SCRIPT).is_err() {
        return SmokeResult { raw: "smoke-test.py couldn't be written".into(), ..Default::default() };
    }
    let wav = storage::root().join("smoke-speech.wav");
    if !wav.exists() {
        // The speech script reads the file name from the environment; set only for that child.
        let script_with_path = format!("$env:SU_WAV = '{}'\n{SPEECH_SCRIPT}", wav.to_string_lossy().replace('\'', "''"));
        proc::powershell(&script_with_path, Duration::from_secs(60), cancel);
    }
    let r = proc::run(python, ["-W".as_ref(), "ignore".as_ref(), script.as_os_str(), wav.as_os_str()], minutes(8), None, cancel);
    parse(&r.output)
}

pub fn parse(output: &str) -> SmokeResult {
    let mut passed = Vec::new();
    let mut failed = Vec::new();
    for line in output.split('\n').map(str::trim) {
        if line.starts_with("PASS ") || line.starts_with("FAIL ") {
            let name = match line.find(':') {
                Some(colon) if colon > 5 => &line[5..colon],
                _ => &line[5..],
            };
            if line.starts_with('P') { passed.push(name.to_string()) } else { failed.push(name.to_string()) }
        }
    }
    SmokeResult { ran: !passed.is_empty() || !failed.is_empty(), passed, failed, raw: output.to_string() }
}

/// Tests that passed before and fail now. Without a baseline, every failure counts.
pub fn new_failures(before: Option<&SmokeResult>, after: &SmokeResult) -> Vec<String> {
    after.failed.iter().filter(|f| before.is_none_or(|b| !b.ran || b.passed.contains(f))).cloned().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_pass_and_fail_lines() {
        let r = parse("PASS pdf: ok\nFAIL whisper: CUDA missing\nnoise\nPASS docx\n");
        assert!(r.ran);
        assert_eq!(r.passed, ["pdf", "docx"]);
        assert_eq!(r.failed, ["whisper"]);
    }

    #[test]
    fn new_failures_need_a_pass_before() {
        let before = parse("PASS pdf\nFAIL whisper\n");
        let after = parse("FAIL pdf\nFAIL whisper\n");
        assert_eq!(new_failures(Some(&before), &after), ["pdf"]);
        assert_eq!(new_failures(None, &after), ["pdf", "whisper"]);
    }
}

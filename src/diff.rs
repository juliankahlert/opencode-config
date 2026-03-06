use similar::TextDiff;

/// Produce a unified-diff string comparing two text blobs.
///
/// * `old_label` / `new_label` — file-name headers shown in the diff
///   (e.g. `"a/opencode.json"`, `"b/opencode.json"`).
/// * `old_content` / `new_content` — the full text to compare.
///
/// Returns the formatted diff as a `String`. If the contents are
/// identical the returned string is empty.
pub fn format_diff(
    old_label: &str,
    old_content: &str,
    new_label: &str,
    new_content: &str,
) -> String {
    if old_content == new_content {
        return String::new();
    }

    let diff = TextDiff::from_lines(old_content, new_content);
    diff.unified_diff()
        .context_radius(3)
        .header(old_label, new_label)
        .to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_identical_content() {
        let content = "line one\nline two\n";
        let result = format_diff("a/file", content, "b/file", content);
        assert!(
            result.is_empty(),
            "identical content should produce empty diff"
        );
    }

    #[test]
    fn test_empty_vs_empty() {
        let result = format_diff("a/file", "", "b/file", "");
        assert!(
            result.is_empty(),
            "empty vs empty should produce empty diff"
        );
    }

    #[test]
    fn test_addition_only() {
        let old = "";
        let new = "first line\nsecond line\n";
        let result = format_diff("/dev/null", old, "b/opencode.json", new);

        assert!(result.contains("--- /dev/null"), "should contain old label");
        assert!(
            result.contains("+++ b/opencode.json"),
            "should contain new label"
        );
        assert!(
            result.contains("+first line"),
            "all lines should be additions"
        );
        assert!(
            result.contains("+second line"),
            "all lines should be additions"
        );
        // No removal lines expected
        assert!(
            !result.contains("-first"),
            "should not contain removal lines"
        );
    }

    #[test]
    fn test_deletion_only() {
        let old = "first line\nsecond line\n";
        let new = "";
        let result = format_diff("a/opencode.json", old, "/dev/null", new);

        assert!(
            result.contains("--- a/opencode.json"),
            "should contain old label"
        );
        assert!(result.contains("+++ /dev/null"), "should contain new label");
        assert!(
            result.contains("-first line"),
            "all lines should be removals"
        );
        assert!(
            result.contains("-second line"),
            "all lines should be removals"
        );
    }

    #[test]
    fn test_single_line_change() {
        let old = "aaa\nbbb\nccc\n";
        let new = "aaa\nBBB\nccc\n";
        let result = format_diff("a/file", old, "b/file", new);

        assert!(result.contains("@@"), "should contain hunk header");
        assert!(result.contains("-bbb"), "should show removed line");
        assert!(result.contains("+BBB"), "should show added line");
    }

    #[test]
    fn test_mixed_change() {
        let old = "line1\nline2\nline3\nline4\nline5\n";
        let new = "line1\nchanged2\nline3\nline4\nadded6\n";
        let result = format_diff("a/file", old, "b/file", new);

        assert!(result.contains("-line2"), "should show removed line2");
        assert!(result.contains("+changed2"), "should show added changed2");
        assert!(result.contains("-line5"), "should show removed line5");
        assert!(result.contains("+added6"), "should show added added6");
    }

    #[test]
    fn test_labels_in_header() {
        let old = "old\n";
        let new = "new\n";
        let result = format_diff("custom/old/path", old, "custom/new/path", new);

        assert!(
            result.contains("--- custom/old/path"),
            "old label should appear in header"
        );
        assert!(
            result.contains("+++ custom/new/path"),
            "new label should appear in header"
        );
    }

    #[test]
    fn test_multiline_json_diff() {
        let old = r#"{
  "agent": {
    "build": {
      "model": "openrouter/openai/gpt-4o",
      "variant": "mini"
    },
    "review": {
      "model": "openrouter/anthropic/claude-sonnet-4"
    }
  }
}
"#;
        let new = r#"{
  "agent": {
    "build": {
      "model": "openrouter/openai/gpt-4o-mini",
      "variant": null
    },
    "review": {
      "model": "openrouter/anthropic/claude-sonnet-4"
    }
  }
}
"#;
        let result = format_diff("a/opencode.json", old, "b/opencode.json", new);

        assert!(result.contains("@@"), "should contain hunk header");
        assert!(
            result.contains("-      \"model\": \"openrouter/openai/gpt-4o\","),
            "should show old model"
        );
        assert!(
            result.contains("+      \"model\": \"openrouter/openai/gpt-4o-mini\","),
            "should show new model"
        );
        assert!(
            result.contains("-      \"variant\": \"mini\""),
            "should show old variant"
        );
        assert!(
            result.contains("+      \"variant\": null"),
            "should show new variant"
        );
        // Context lines should include unchanged surrounding lines
        assert!(
            result.contains(" {"),
            "should include context around changes"
        );
    }

    #[test]
    fn test_no_trailing_newline() {
        // Both inputs lack trailing newline
        let old = "line1\nline2";
        let new = "line1\nline3";
        let result = format_diff("a/file", old, "b/file", new);

        assert!(result.contains("-line2"), "should show removed line");
        assert!(result.contains("+line3"), "should show added line");
        // The diff should still be well-formed
        assert!(result.contains("@@"), "should contain hunk header");
    }

    #[test]
    fn test_trailing_newline_only_in_new() {
        let old = "same content";
        let new = "same content\n";
        let result = format_diff("a/file", old, "b/file", new);

        // Content differs only by trailing newline — should produce a diff
        assert!(
            !result.is_empty(),
            "trailing newline difference should produce a diff"
        );
    }

    #[test]
    fn test_unicode_handling() {
        let old = "Hello, world!\n日本語テスト\nEmoji: 🦀\n";
        let new = "Hello, world!\n日本語テスト変更\nEmoji: 🦀🔥\n";
        let result = format_diff("a/file", old, "b/file", new);

        assert!(
            result.contains("-日本語テスト"),
            "should show removed unicode line"
        );
        assert!(
            result.contains("+日本語テスト変更"),
            "should show added unicode line"
        );
        assert!(
            result.contains("-Emoji: 🦀"),
            "should show removed emoji line"
        );
        assert!(
            result.contains("+Emoji: 🦀🔥"),
            "should show added emoji line"
        );
    }
}

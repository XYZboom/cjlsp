use std::fs;
use std::process::{Command, Output};

fn source_file(label: &str) -> std::path::PathBuf {
    let path = std::env::temp_dir().join(format!(
        "cj-frontend-diagnostic-format-{}-{label}.cj",
        std::process::id()
    ));
    fs::write(&path, "func f( { }\n").unwrap();
    path
}

fn run(args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_cj-frontend"))
        .args(args)
        .output()
        .unwrap()
}

#[test]
fn explicit_text_is_byte_identical_to_default() {
    let path = source_file("text");
    let path_text = path.to_str().unwrap();
    let default = run(&[path_text]);
    let explicit = run(&["--diagnostic-format=text", path_text]);
    assert_eq!(default.stdout, explicit.stdout);
    assert_eq!(default.stderr, explicit.stderr);
    fs::remove_file(path).unwrap();
}

#[test]
fn json_format_accepts_joined_and_separate_arguments() {
    let path = source_file("json");
    let path_text = path.to_str().unwrap();
    let joined = run(&["--diagnostic-format=json", path_text]);
    let separate = run(&["--diagnostic-format", "json", path_text]);
    assert_eq!(joined.stderr, separate.stderr);

    let json: serde_json::Value = serde_json::from_slice(&joined.stderr).unwrap();
    assert_eq!(json["schemaVersion"], 1);
    let diagnostics = json["diagnostics"].as_array().unwrap();
    assert!(!diagnostics.is_empty());
    let parser_diag = diagnostics
        .iter()
        .find(|diag| diag["category"] == "parser")
        .unwrap();
    assert!(parser_diag["code"].is_string());
    assert!(parser_diag["location"]["range"]["start"]["line"].is_number());
    assert!(!String::from_utf8(joined.stderr).unwrap().contains(":null"));
    fs::remove_file(path).unwrap();
}

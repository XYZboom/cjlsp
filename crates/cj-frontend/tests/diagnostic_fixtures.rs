use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

fn normalize_value(val: &mut serde_json::Value) {
    match val {
        serde_json::Value::Object(map) => {
            for (k, v) in map.iter_mut() {
                if k == "file" {
                    if let serde_json::Value::String(s) = v {
                        if let Some(file_name) = Path::new(s).file_name().and_then(|f| f.to_str()) {
                            *v = serde_json::Value::String(file_name.to_string());
                        }
                    }
                } else {
                    normalize_value(v);
                }
            }
        }
        serde_json::Value::Array(arr) => {
            for item in arr {
                normalize_value(item);
            }
        }
        _ => {}
    }
}

fn run_fixture(fixture_dir: &Path) {
    let input_path = fixture_dir.join("input.cj");
    let expected_path = fixture_dir.join("expected.json");

    assert!(
        input_path.exists(),
        "fixture {} missing input.cj",
        fixture_dir.display()
    );
    assert!(
        expected_path.exists(),
        "fixture {} missing expected.json",
        fixture_dir.display()
    );

    let output = Command::new(env!("CARGO_BIN_EXE_cj-frontend"))
        .arg("--diagnostic-format=json")
        .arg(&input_path)
        .output()
        .unwrap_or_else(|e| panic!("failed to run cj-frontend on {}: {e}", input_path.display()));

    assert!(
        output.status.success(),
        "cj-frontend exited with non-zero status on {}: stderr={}",
        input_path.display(),
        String::from_utf8_lossy(&output.stderr)
    );

    let actual_raw = String::from_utf8(output.stderr).expect("invalid utf-8 in cj-frontend stderr");
    let mut actual_json: serde_json::Value = serde_json::from_str(&actual_raw).unwrap_or_else(|e| {
        panic!(
            "failed to parse JSON from cj-frontend stderr for {}: {e}\noutput: {actual_raw}",
            input_path.display()
        )
    });

    let expected_raw = fs::read_to_string(&expected_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", expected_path.display()));
    let mut expected_json: serde_json::Value =
        serde_json::from_str(&expected_raw).unwrap_or_else(|e| {
            panic!(
                "failed to parse JSON from expected.json for {}: {e}",
                expected_path.display()
            )
        });

    normalize_value(&mut actual_json);
    normalize_value(&mut expected_json);

    // Validate schemaVersion
    assert_eq!(
        actual_json.get("schemaVersion"),
        expected_json.get("schemaVersion"),
        "fixture {} schemaVersion mismatch",
        fixture_dir.display()
    );

    // Validate each diagnostic entry in actual JSON has required schema fields
    if let Some(diagnostics) = actual_json.get("diagnostics").and_then(|d| d.as_array()) {
        for (idx, diag) in diagnostics.iter().enumerate() {
            assert!(
                diag.get("severity").is_some(),
                "fixture {} diagnostic[{idx}] missing severity",
                fixture_dir.display()
            );
            assert!(
                diag.get("message").is_some(),
                "fixture {} diagnostic[{idx}] missing message",
                fixture_dir.display()
            );
            assert!(
                diag.get("location").is_some(),
                "fixture {} diagnostic[{idx}] missing location",
                fixture_dir.display()
            );
            let loc = &diag["location"];
            assert!(
                loc.get("file").is_some() && loc.get("range").is_some(),
                "fixture {} diagnostic[{idx}] invalid location structure",
                fixture_dir.display()
            );
            let range = &loc["range"];
            assert!(
                range.get("start").is_some() && range.get("end").is_some(),
                "fixture {} diagnostic[{idx}] range missing start/end",
                fixture_dir.display()
            );
        }
    }

    assert_eq!(
        actual_json,
        expected_json,
        "fixture {} mismatch!\nActual:\n{}\nExpected:\n{}",
        fixture_dir.display(),
        serde_json::to_string_pretty(&actual_json).unwrap(),
        serde_json::to_string_pretty(&expected_json).unwrap()
    );
}

#[test]
fn test_all_diagnostic_fixtures() {
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    // crates/cj-frontend -> workspace root -> tests/diagnostics
    let workspace_root = manifest_dir
        .parent()
        .and_then(|p| p.parent())
        .expect("failed to determine workspace root");
    let fixtures_root = workspace_root.join("tests").join("diagnostics");

    assert!(
        fixtures_root.is_dir(),
        "tests/diagnostics directory does not exist at {}",
        fixtures_root.display()
    );

    let mut count = 0;
    let mut entries: Vec<_> = fs::read_dir(&fixtures_root)
        .unwrap()
        .filter_map(|e| e.ok())
        .filter(|e| e.path().is_dir())
        .collect();
    entries.sort_by_key(|e| e.path());

    for entry in entries {
        let fixture_dir = entry.path();
        run_fixture(&fixture_dir);
        count += 1;
    }

    assert!(
        count >= 5,
        "expected at least 5 diagnostic fixtures, found {count}"
    );
}

// cj-lsp: LSP server entry — LSPServer binary over stdio.
//
// Protocol: LSP (JSON-RPC 2.0) over stdio with Content-Length framing.
// Test harness drives us with: LSPServer --test --disableAutoImport --enable-log=true
// (cjlsp/lsp_test.py). Messages are handled generically via serde_json::Value
// because LSP params are large and variant; we dispatch on method + id.

use serde_json::{json, Value};
use std::io::{self, BufRead, Write};
use std::process::ExitCode;

mod server;
use server::LspServer;

/// Frames: "Content-Length: <N>\r\n\r\n<body>"
const MAX_FRAME: usize = 64 * 1024 * 1024; // 64 MiB safety cap

fn read_message(reader: &mut impl BufRead) -> io::Result<Option<Value>> {
    // Skip stray blank lines before the header block. The test harness
    // (lsp_test.py) emits an extra blank line between frames, which a strict
    // "blank line ends headers" parser would misread as a missing length.
    let mut len: Option<usize> = None;
    loop {
        let mut header = String::new();
        let n = reader.read_line(&mut header)?;
        if n == 0 {
            return Ok(None); // EOF
        }
        let line = header.trim_end();
        if line.is_empty() {
            if len.is_some() {
                break; // blank line terminates the header block
            }
            continue; // leading blank line — skip, keep reading
        }
        if let Some(rest) = line.strip_prefix("Content-Length:") {
            len = rest.trim().parse::<usize>().ok();
        }
    }
    let len =
        len.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing Content-Length"))?;
    if len > MAX_FRAME {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "frame too large",
        ));
    }
    let mut body = vec![0u8; len];
    reader.read_exact(&mut body)?;
    let text = String::from_utf8(body)
        .map_err(|_| io::Error::new(io::ErrorKind::InvalidData, "non-UTF8 body"))?;
    if text.trim().is_empty() {
        return Ok(None); // empty frame — treat as EOF-ish, stop reading
    }
    Ok(Some(serde_json::from_str(&text).map_err(|e| {
        io::Error::new(io::ErrorKind::InvalidData, format!("bad JSON: {e}"))
    })?))
}

fn write_message(out: &mut impl Write, msg: &Value) -> io::Result<()> {
    let body = serde_json::to_string(msg).unwrap_or_else(|_| "{}".to_string());
    write!(out, "Content-Length: {}\r\n\r\n{}", body.len(), body)?;
    out.flush()
}

fn main() -> ExitCode {
    // Parse flags (test harness passes --test --disableAutoImport --enable-log=true).
    let mut test_mode = false;
    for arg in std::env::args().skip(1) {
        if arg == "--test" {
            test_mode = true;
        }
    }

    let stdin = io::stdin();
    let stdout = io::stdout();
    let mut reader = io::BufReader::new(stdin.lock());
    let mut writer = io::BufWriter::new(stdout.lock());

    let mut server = LspServer::new(test_mode);

    loop {
        let msg = match read_message(&mut reader) {
            Ok(Some(m)) => m,
            Ok(None) => break, // EOF
            Err(e) => {
                eprintln!("[lsp] read error: {e}");
                break;
            }
        };
        let method = msg
            .get("method")
            .and_then(Value::as_str)
            .unwrap_or("")
            .to_string();
        let has_id = msg.get("id").is_some();

        // Handle shutdown/exit lifecycle before dispatching.
        if method == "exit" {
            server.handle_exit();
            break;
        }
        if method == "shutdown" {
            let result = server.handle_shutdown();
            if has_id {
                let _ = write_message(
                    &mut writer,
                    &json!({"jsonrpc": "2.0", "id": msg["id"], "result": result}),
                );
            }
            continue;
        }

        // Dispatch: requests (with id) get responses; notifications get none.
        if has_id {
            let result =
                server.dispatch(&method, msg.get("params").cloned().unwrap_or(Value::Null));
            let response = json!({
                "jsonrpc": "2.0",
                "id": msg["id"],
                "result": result
            });
            let _ = write_message(&mut writer, &response);
        } else {
            let notifications =
                server.notify(&method, msg.get("params").cloned().unwrap_or(Value::Null));
            for n in notifications {
                let _ = write_message(&mut writer, &n);
            }
        }
    }

    ExitCode::SUCCESS
}

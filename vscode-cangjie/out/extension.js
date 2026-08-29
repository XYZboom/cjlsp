'use strict';
/*
 * vscode-cangjie: Cangjie language extension for VSCode.
 *
 * A thin client that launches the Rust LSPServer binary (cj-lsp crate) and
 * speaks LSP over stdio. The server is fully standard: JSON-RPC 2.0 with
 * Content-Length framing (see cj-lsp/src/main.rs).
 *
 * Served features (match cj-lsp/src/server.rs dispatch):
 *   - textDocument/completion
 *   - textDocument/hover
 *   - textDocument/definition
 *   - textDocument/references
 *   - textDocument/publishDiagnostics (derived from didOpen / didChange)
 *
 * The server's initialize response advertises more (semanticTokens,
 * documentSymbol, rename, documentHighlight) than dispatch actually
 * implements; the middleware below masks those so the VSCode UX stays
 * error-free (no "Request textDocument/semanticTokens/full failed" noise).
 */

const vscode = require('vscode');
const {
  LanguageClient,
  TransportKind,
  RevealOutputChannelOn,
} = require('vscode-languageclient/node');

// Bundled server: the release LSPServer ships inside the extension, with one
// build per platform (bin/linux, bin/win32, bin/darwin), so the extension
// works out of the box with zero config on Windows / macOS / Linux. An
// explicit cangjie.lsp.serverPath setting or the CANGJIE_LSPSERVER env var
// overrides it (e.g. to point at a freshly built debug binary).
const SERVER_DEFAULT = '';

/** @type {import('vscode-languageclient/node').LanguageClient | null} */
let client = null;

function platformServerBinary() {
  // Returns the bundled binary path for the current platform, or null if we
  // don't ship a build for it.
  const path = require('path');
  const { platform } = process;
  let subdir = null;
  let exe = 'LSPServer';
  if (platform === 'win32') {
    subdir = 'win32';
    exe = 'LSPServer.exe';
  } else if (platform === 'linux') {
    subdir = 'linux';
  } else if (platform === 'darwin') {
    subdir = 'darwin';
  } else {
    return null;
  }
  return path.join(__dirname, '..', 'bin', subdir, exe);
}

function serverCommand(context) {
  const settings = vscode.workspace.getConfiguration('cangjie');
  const fromEnv = process.env.CANGJIE_LSPSERVER || '';
  const configured = settings.get('lsp.serverPath', '') || fromEnv || SERVER_DEFAULT;
  if (configured) {
    return configured;
  }
  // Bundled copy inside the extension directory.
  return platformServerBinary();
}

/**
 * @param {import('vscode').ExtensionContext} context
 */
function activate(context) {
  const fs = require('fs');
  const command = serverCommand(context);

  // Always create the output channel up front (even if the binary is missing
  // or the server later fails to start), so "View -> Output -> Cangjie
  // Language Server" is always present and shows the diagnostics.
  const outputChannel = vscode.window.createOutputChannel('Cangjie LSP (cj-lang)');

  if (!command || !fs.existsSync(command)) {
    const msg = "Cangjie: LSPServer binary not found at '" + (command || '(none)') +
      "'. This extension bundles builds for Windows/Linux/macOS; if yours is " +
      "missing, set 'cangjie.lsp.serverPath' (or the CANGJIE_LSPSERVER env var) " +
      'to a built cj-lsp binary (cargo build -p cj-lsp).';
    outputChannel.appendLine('[vscode-cangjie] ' + msg);
    outputChannel.show(true);
    vscode.window.showErrorMessage(msg);
    return;
  }
  outputChannel.appendLine('[vscode-cangjie] server command = ' + command);

  const settings = vscode.workspace.getConfiguration('cangjie');
  const extraArgs = /** @type {string[]} */ (settings.get('lsp.extraArgs', []));
  const env = Object.assign({}, process.env, settings.get('lsp.env', {}));

  // --test / --disableAutoImport mirror the official cjlsp harness launch
  // (tools/lsp_cov.py). --enable-log=true is added only in debug mode so
  // normal runs stay quiet.
  const baseArgs = ['--test', '--disableAutoImport'];
  const serverOptions = {
    run: {
      command,
      args: baseArgs.concat(extraArgs),
      options: { env },
      transport: TransportKind.stdio,
    },
    debug: {
      command,
      args: baseArgs.concat(['--enable-log=true']).concat(extraArgs),
      options: { env },
      transport: TransportKind.stdio,
    },
  };

  const clientOptions = {
    documentSelector: [{ scheme: 'file', language: 'cangjie' }],
    // Use the channel we created ourselves so it exists even on failure.
    outputChannel,
    revealOutputChannelOn: RevealOutputChannelOn.Never,
    synchronize: { configurationSection: 'cangjie' },
    // Mask capabilities the server advertises but does not implement.
    middleware: {
      provideDocumentSemanticTokens: () => null,
      provideDocumentSymbols: () => null,
      provideRenameEdits: () => null,
      provideDocumentHighlights: () => null,
    },
  };

  client = new LanguageClient(
    'cangjie-lsp-rust',
    'Cangjie LSP (cj-lang)',
    serverOptions,
    clientOptions
  );
  // vscode-languageclient 10.x: start() returns Promise<void>; teardown is
  // handled by deactivate() -> client.stop(). Surface any start failure
  // (bad binary, spawn error) instead of failing silently.
  client.start().then(
    () => {
      outputChannel.appendLine('[vscode-cangjie] client started');
      console.log('[vscode-cangjie] client started, command = ' + command);
      vscode.window.setStatusBarMessage('Cangjie LSP: started', 3000);
    },
    (err) => {
      const msg = 'Cangjie LSP failed to start: ' + (err && err.message ? err.message : err);
      outputChannel.appendLine('[vscode-cangjie] ERROR ' + msg);
      outputChannel.show(true);
      console.error('[vscode-cangjie]', msg);
      vscode.window.showErrorMessage(msg);
    }
  );
}

function deactivate() {
  if (client) {
    return client.stop();
  }
  return undefined;
}

module.exports = { activate, deactivate };

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

const SERVER_DEFAULT = '/root/Code/cangjie/cj-lang/target/debug/LSPServer';

/** @type {import('vscode-languageclient/node').LanguageClient | null} */
let client = null;

function serverCommand() {
  const settings = vscode.workspace.getConfiguration('cangjie');
  const fromEnv = process.env.CANGJIE_LSPSERVER || '';
  return settings.get('lsp.serverPath', '') || fromEnv || SERVER_DEFAULT;
}

/**
 * @param {import('vscode').ExtensionContext} context
 */
function activate(context) {
  const fs = require('fs');
  const command = serverCommand();
  if (!fs.existsSync(command)) {
    vscode.window.showErrorMessage(
      "Cangjie: LSPServer binary not found at '" + command +
      "'. Set 'cangjie.lsp.serverPath' (or the CANGJIE_LSPSERVER env var) to " +
      'a built cj-lsp binary (cargo build -p cj-lsp).'
    );
    return;
  }

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
    // The client creates its own log output channel ("Cangjie Language Server").
    outputChannelName: 'Cangjie Language Server',
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
    'cangjie-lsp',
    'Cangjie Language Server',
    serverOptions,
    clientOptions
  );
  // vscode-languageclient 10.x: start() returns Promise<void>; teardown is
  // handled by deactivate() -> client.stop().
  client.start();
}

function deactivate() {
  if (client) {
    return client.stop();
  }
  return undefined;
}

module.exports = { activate, deactivate };

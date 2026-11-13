'use strict';

import * as vscode from 'vscode';
import {
  LanguageClient as LspLanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
  RevealOutputChannelOn
} from 'vscode-languageclient/node';
import * as path from 'path';
import * as fs from 'fs';
import * as os from 'os';

let client: LspLanguageClient | undefined;
let outputChannel: vscode.OutputChannel;

export async function activate(context: vscode.ExtensionContext) {
  outputChannel = vscode.window.createOutputChannel('EraBasic LSP');
  context.subscriptions.push(outputChannel);

  // Register commands
  context.subscriptions.push(
    vscode.commands.registerCommand('erabasic-lsp.restartServer', () => restartServer(context)),
    vscode.commands.registerCommand('erabasic-lsp.showOutput', () => outputChannel.show())
  );

  // Auto-start on .erb/.erh files
  const disposable = vscode.workspace.onDidOpenTextDocument(doc => {
    if (doc.languageId === 'erbasic') {
      startServer(context);
    }
  });
  context.subscriptions.push(disposable);

  // Check if any open .erb/.erh files
  for (const doc of vscode.workspace.textDocuments) {
    if (doc.languageId === 'erbasic') {
      startServer(context);
      break;
    }
  }

  // Configuration change handler
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration(e => {
      if (e.affectsConfiguration('erabasic-lsp')) {
        restartServer(context);
      }
    })
  );
}

async function startServer(context: vscode.ExtensionContext): Promise<void> {
  if (client) {
    return; // Already running
  }

  const config = vscode.workspace.getConfiguration('erabasic-lsp');
  if (!config.get<boolean>('enable', true)) {
    return;
  }

  let serverPath = config.get<string>('serverPath', '') || '';

  // Auto-detect server binary
  if (!serverPath) {
    serverPath = await findServerBinary(context) || '';
  }

  if (!serverPath || !fs.existsSync(serverPath)) {
    const msg = `EraBasic LSP server not found at: ${serverPath || 'auto-detect failed'}`;
    outputChannel.appendLine(`[Error] ${msg}`);
    vscode.window.showErrorMessage(msg, 'Configure Path').then(choice => {
      if (choice === 'Configure Path') {
        vscode.commands.executeCommand('workbench.action.openSettings', 'erabasic-lsp.serverPath');
      }
    });
    return;
  }

  outputChannel.appendLine(`[Info] Starting EraBasic LSP from: ${serverPath}`);

  const runOptions: ServerOptions = {
    run: { command: serverPath, transport: TransportKind.stdio },
    debug: { command: serverPath, transport: TransportKind.stdio, options: { env: { ...process.env, RUST_LOG: 'debug' } } }
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [
      { scheme: 'file', language: 'erbasic' }
    ],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher('**/*.{erb,erh}'),
      configurationSection: 'erabasic-lsp'
    },
    revealOutputChannelOn: RevealOutputChannelOn.Never,
    outputChannel,
    traceOutputChannel: outputChannel,
    initializationOptions: {
      dialect: config.get<string>('dialect', 'auto')
    }
  };

  client = new LspLanguageClient('erabasic-lsp', 'EraBasic LSP', runOptions, clientOptions);

  // Handle server errors
  client.onDidChangeState(({ newState }) => {
    const stateStr = stateToString(newState);
    outputChannel.appendLine(`[Info] Server state: ${stateStr}`);
    if (newState === 3) { // Stopped
      outputChannel.appendLine('[Warning] Server stopped unexpectedly');
      vscode.window.showWarningMessage('EraBasic LSP server stopped. Check output for details.');
    }
  });

  try {
    await client.start();
    outputChannel.appendLine('[Info] EraBasic LSP started successfully');
  } catch (err) {
    outputChannel.appendLine(`[Error] Failed to start server: ${err}`);
    vscode.window.showErrorMessage(`Failed to start EraBasic LSP: ${err}`);
    client = undefined;
  }
}

async function restartServer(context: vscode.ExtensionContext): Promise<void> {
  if (client) {
    outputChannel.appendLine('[Info] Restarting EraBasic LSP...');
    await client.stop();
    client = undefined;
  }
  await startServer(context);
}

async function findServerBinary(context: vscode.ExtensionContext): Promise<string | undefined> {
  const candidates: string[] = [];

  // 1. Configured path
  const configPath = vscode.workspace.getConfiguration('erabasic-lsp').get<string>('serverPath', '');
  if (configPath) candidates.push(configPath);

  // 2. Bundled with extension (if we bundle later)
  const bundled = path.join(context.extensionPath, 'server', 'erabasic-lsp');
  candidates.push(bundled);
  candidates.push(bundled + '.exe');

  // 3. Cargo target directories (development)
  const workspaceFolders = vscode.workspace.workspaceFolders;
  if (workspaceFolders) {
    for (const folder of workspaceFolders) {
      const wsPath = folder.uri.fsPath;
      candidates.push(path.join(wsPath, 'target', 'release', 'erabasic-lsp'));
      candidates.push(path.join(wsPath, 'target', 'debug', 'erabasic-lsp'));
      candidates.push(path.join(wsPath, 'erabasic-rust-lsp', 'target', 'release', 'erabasic-lsp'));
      candidates.push(path.join(wsPath, 'erabasic-rust-lsp', 'target', 'debug', 'erabasic-lsp'));
    }
  }

  // 4. Global cargo install
  const cargoBin = process.env.CARGO_HOME ? path.join(process.env.CARGO_HOME, 'bin', 'erabasic-lsp')
    : path.join(os.homedir(), '.cargo', 'bin', 'erabasic-lsp');
  candidates.push(cargoBin);
  candidates.push(cargoBin + '.exe');

  // 5. PATH
  candidates.push('erabasic-lsp');

  for (const candidate of candidates) {
    try {
      if (fs.existsSync(candidate) || candidate === 'erabasic-lsp') {
        // Verify it's executable
        if (candidate !== 'erabasic-lsp') {
          fs.accessSync(candidate, fs.constants.X_OK);
        }
        return candidate;
      }
    } catch {
      // Continue to next candidate
    }
  }

  return undefined;
}

export function deactivate(): Thenable<void> | undefined {
  if (client) {
    return client.stop();
  }
  return undefined;
}

// Helper to get state string
function stateToString(state: number): string {
  switch (state) {
    case 0: return 'Stopped';
    case 1: return 'Starting';
    case 2: return 'Running';
    case 3: return 'Stopped';
    default: return 'Unknown';
  }
}
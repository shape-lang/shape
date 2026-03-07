import { execSync, spawn } from 'child_process';
import { workspace, window, ExtensionContext, ProgressLocation } from 'vscode';
import {
    LanguageClient,
    LanguageClientOptions,
    ServerOptions,
} from 'vscode-languageclient/node';

let client: LanguageClient;

function isCommandAvailable(command: string): boolean {
    try {
        execSync(`${command} --version`, { stdio: 'ignore' });
        return true;
    } catch {
        return false;
    }
}

function startLspClient() {
    const serverOptions: ServerOptions = {
        command: 'shape-lsp',
        args: [],
    };

    const clientOptions: LanguageClientOptions = {
        documentSelector: [{ scheme: 'file', language: 'shape' }],
        synchronize: {
            fileEvents: workspace.createFileSystemWatcher('**/*.shape'),
        },
    };

    client = new LanguageClient(
        'shapeLsp',
        'Shape Language Server',
        serverOptions,
        clientOptions
    );

    client.start();
}

async function installShapeLsp(): Promise<boolean> {
    return window.withProgress(
        {
            location: ProgressLocation.Notification,
            title: 'Installing shape-lsp...',
            cancellable: true,
        },
        (_progress, token) => {
            return new Promise<boolean>((resolve) => {
                const proc = spawn('cargo', ['install', 'shape-lsp'], {
                    stdio: 'ignore',
                });

                token.onCancellationRequested(() => {
                    proc.kill();
                    resolve(false);
                });

                proc.on('close', (code: number | null) => {
                    resolve(code === 0);
                });

                proc.on('error', () => {
                    resolve(false);
                });
            });
        }
    );
}

export async function activate(context: ExtensionContext) {
    if (isCommandAvailable('shape-lsp')) {
        startLspClient();
        return;
    }

    const hasCargo = isCommandAvailable('cargo');

    const choices = hasCargo
        ? ['Install via cargo', 'Not now']
        : ['Show install instructions', 'Not now'];

    const choice = await window.showWarningMessage(
        'shape-lsp is not installed. Install it for full language support (completions, diagnostics, hover, go-to-definition).',
        ...choices
    );

    if (choice === 'Install via cargo') {
        const success = await installShapeLsp();
        if (success) {
            window.showInformationMessage('shape-lsp installed successfully.');
            startLspClient();
        } else {
            window.showErrorMessage(
                'Failed to install shape-lsp. Try manually: cargo install shape-lsp'
            );
        }
    } else if (choice === 'Show install instructions') {
        window.showInformationMessage(
            'Install Rust (https://rustup.rs), then run: cargo install shape-lsp'
        );
    }
}

export function deactivate(): Thenable<void> | undefined {
    if (!client) {
        return undefined;
    }
    return client.stop();
}

// Starts `yavs run --dap=stdio` for a "yavs" debug configuration, or attaches to a port.
'use strict';

const vscode = require('vscode');
const { spawnSync } = require('child_process');

/** The `yavs` arguments common to analyze and run. */
function libArgs(c) {
  const a = [];
  if (c.std) a.push(`--std=${c.std}`);
  a.push(`--work=${c.work || 'work:.yavs/work'}`);
  for (const m of c.libraries || []) a.push(`--map=${m}`);
  return a;
}

/** `yavs run` arguments of a launch configuration. */
function runArgs(c) {
  const a = ['run', ...libArgs(c)];
  for (const [k, v] of Object.entries(c.generics || {})) a.push(`-g${k}=${v}`);
  a.push(...(c.args || []), '--dap=stdio', c.top);
  return a;
}

class Factory {
  createDebugAdapterDescriptor(session) {
    const c = session.configuration;
    if (c.port) return new vscode.DebugAdapterServer(c.port, '127.0.0.1');
    const exe = c.yavsPath || 'yavs';
    const cwd = c.cwd || (vscode.workspace.workspaceFolders || [])[0]?.uri.fsPath;
    if ((c.sources || []).length > 0) {
      const r = spawnSync(exe, ['analyze', ...libArgs(c), ...c.sources], { cwd, encoding: 'utf8' });
      if (r.error || r.status !== 0) {
        const msg = r.error ? r.error.message : (r.stderr || r.stdout);
        vscode.window.showErrorMessage(`yavs analyze failed: ${msg}`);
        return undefined;
      }
    }
    return new vscode.DebugAdapterExecutable(exe, runArgs(c), { cwd });
  }
}

function activate(context) {
  context.subscriptions.push(
    vscode.debug.registerDebugAdapterDescriptorFactory('yavs', new Factory()));
}

function deactivate() {}

module.exports = { activate, deactivate, runArgs };

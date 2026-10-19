// Inside VS Code: set a breakpoint in tb.vhd, start "Debug tb" from launch.json, and check
// that the session stops there with a frame VS Code can open, then runs to the end.
'use strict';

const assert = require('assert');
const vscode = require('vscode');

function waitFor(what, ms, register) {
  return new Promise((resolve, reject) => {
    const t = setTimeout(() => reject(new Error(`timeout waiting for ${what}`)), ms);
    register((v) => {
      clearTimeout(t);
      resolve(v);
    });
  });
}

async function run() {
  const folder = vscode.workspace.workspaceFolders[0];
  const tb = vscode.Uri.joinPath(folder.uri, 'tb.vhd');
  const doc = await vscode.workspace.openTextDocument(tb);
  assert.equal(doc.languageId, 'vhdl');
  // Line 20, `i := twice(k);` (0-based 19).
  vscode.debug.addBreakpoints([
    new vscode.SourceBreakpoint(new vscode.Location(tb, new vscode.Position(19, 0))),
  ]);

  const stops = [];
  let onStop;
  vscode.debug.registerDebugAdapterTrackerFactory('yavs', {
    createDebugAdapterTracker: () => ({
      onDidSendMessage: (m) => {
        if (m.type === 'event' && m.event === 'stopped') {
          stops.push(m.body);
          if (onStop) onStop(m.body);
        }
      },
    }),
  });
  const stopped = waitFor('stopped', 60000, (r) => (onStop = r));
  const started = await vscode.debug.startDebugging(folder, 'Debug tb');
  assert.ok(started, 'startDebugging failed');
  const s = await stopped;
  assert.equal(s.reason, 'breakpoint');

  const session = vscode.debug.activeDebugSession;
  const st = await session.customRequest('stackTrace', { threadId: s.threadId });
  const top = st.stackFrames[0];
  assert.equal(top.line, 20);
  assert.equal(vscode.Uri.file(top.source.path).toString(), tb.toString());
  // VS Code shows the stop: the active stack item is that frame.
  await new Promise((r) => setTimeout(r, 500));
  const item = vscode.debug.activeStackItem;
  assert.ok(item && item.frameId === top.id, `active stack item ${JSON.stringify(item)}`);

  const ended = waitFor('end of session', 60000, (r) => vscode.debug.onDidTerminateDebugSession(r));
  vscode.debug.removeBreakpoints(vscode.debug.breakpoints);
  await session.customRequest('continue', { threadId: s.threadId });
  await ended;
  assert.equal(stops.length, 1);
}

module.exports = { run };

// Inside VS Code: set a breakpoint in tb.vhd, start "Debug tb" from launch.json, and check
// that the session stops there with a frame VS Code can open, steps one delta with the yavs
// command, stops on a signal value, then runs to the end.
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
  // Line 20, `i := twice(k);` (0-based 19), with a condition VS Code sends, and one on the
  // function body that never holds.
  vscode.debug.addBreakpoints([
    new vscode.SourceBreakpoint(new vscode.Location(tb, new vscode.Position(19, 0)), true, 'k = 1'),
    new vscode.SourceBreakpoint(new vscode.Location(tb, new vscode.Position(11, 0)), true, 'x > 9'),
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

  // The Variables view's scopes, and a composite expanded the way VS Code pages it.
  const scopeNames = (await session.customRequest('scopes', { frameId: top.id })).scopes.map((x) => x.name);
  assert.deepEqual(scopeNames, ['Locals', 'Signals', 'Generics', 'Hierarchy']);
  const vv = await session.customRequest('evaluate', { expression: 'v', frameId: top.id });
  const el = await session.customRequest('variables', {
    variablesReference: vv.variablesReference, filter: 'indexed', start: 0, count: vv.indexedVariables,
  });
  assert.deepEqual(el.variables.map((x) => x.value), ["'0'", "'1'", "'0'", "'1'"]);

  // "Set Value" on a local, then the extension's force command (as its context menu entry
  // on a signal calls it, with the answers given).
  const sc = await session.customRequest('scopes', { frameId: top.id });
  const set = await session.customRequest('setVariable', {
    variablesReference: sc.scopes[0].variablesReference, name: 'i', value: '5',
  });
  assert.equal(set.value, '5');
  await vscode.commands.executeCommand('yavs.force', { signal: 'v', mode: 'in', value: '"1111"' });

  // The command palette's "yavs: Step to the next delta" ends the cycle stopped in.
  const stepped = waitFor('stopped after the delta step', 60000, (r) => (onStop = r));
  await vscode.commands.executeCommand('yavs.stepDelta');
  const d = await stepped;
  assert.equal(d.reason, 'step');
  assert.match(d.description, /^step at 0 fs, delta 0$/);
  // The force is made in the next cycle and holds; release it again.
  const step2 = waitFor('stopped after the second delta step', 60000, (r) => (onStop = r));
  await vscode.commands.executeCommand('yavs.stepDelta');
  await step2;
  const ev = await session.customRequest('evaluate', { expression: ':tb:v' });
  assert.equal(ev.result, '"1111"');
  await vscode.commands.executeCommand('yavs.release', { signal: ':tb:v', mode: 'in' });

  // A signal value breakpoint (what "yavs: Break when a signal equals..." sends), without the
  // line breakpoint, which the process would hit first.
  vscode.debug.removeBreakpoints(vscode.debug.breakpoints);
  await new Promise((r) => setTimeout(r, 500));
  const hit = waitFor('stopped on the signal', 60000, (r) => (onStop = r));
  await session.customRequest('yavs/setSignalBreakpoints', {
    breakpoints: [{ signal: ':tb:cnt', value: '6' }],
  });
  await session.customRequest('continue', { threadId: 1 });
  const h = await hit;
  assert.equal(h.reason, 'data breakpoint');
  assert.equal(h.text, ':tb:cnt = 6');
  await session.customRequest('yavs/setSignalBreakpoints', { breakpoints: [] });

  const ended = waitFor('end of session', 60000, (r) => vscode.debug.onDidTerminateDebugSession(r));
  await session.customRequest('continue', { threadId: 1 });
  await ended;
  assert.equal(stops.length, 4);
}

module.exports = { run };

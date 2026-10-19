// Runs suite.js inside a real VS Code with this extension on a copy of ../../sample.
// Usage: node test/vscode/run.js path/to/yavs [path/to/code]
// Without a code path, @vscode/test-electron downloads VS Code into .vscode-test.
'use strict';

const fs = require('fs');
const os = require('os');
const path = require('path');
const { runTests } = require('@vscode/test-electron');

async function main() {
  const yavs = path.resolve(process.argv[2] || 'yavs');
  const ws = fs.mkdtempSync(path.join(os.tmpdir(), 'yavs-vscode-ws-'));
  fs.cpSync(path.join(__dirname, '..', '..', 'sample'), ws, { recursive: true });
  const env = { PATH: `${path.dirname(yavs)}${path.delimiter}${process.env.PATH}` };
  try {
    await runTests({
      vscodeExecutablePath: process.argv[3],
      extensionDevelopmentPath: path.join(__dirname, '..', '..'),
      extensionTestsPath: path.join(__dirname, 'suite.js'),
      extensionTestsEnv: env,
      // No window: the Chromium headless platform.
      launchArgs: [ws, '--disable-extensions', '--disable-gpu', '--ozone-platform=headless',
        `--user-data-dir=${path.join(ws, '.user')}`],
    });
    console.log('ok: breakpoint hit in VS Code');
  } catch (e) {
    console.error(e);
    process.exitCode = 1;
  } finally {
    fs.rmSync(ws, { recursive: true, force: true });
  }
}

main();

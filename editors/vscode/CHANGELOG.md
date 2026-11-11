# Changelog

## 0.19.1 (2026-11-11)

- Needs yavs 0.19.1 or later. All changes: the yavs 0.19.1 release notes.

## 0.19.0 (2026-11-10)

- Needs yavs 0.19.0 or later. All changes: the yavs 0.19.0 release notes.

## 0.18.0 (2026-11-09)

- Needs yavs 0.18.0 or later. All changes: the yavs 0.18.0 release notes.

## 0.17.0 (2026-11-08)

- Needs yavs 0.17.0 or later. All changes: the yavs 0.17.0 release notes.

## 0.16.1 (2026-11-06)

- Needs yavs 0.16.1 or later. All changes: the yavs 0.16.1 release notes.

## 0.16.0 (2026-11-06)

- Needs yavs 0.16.0 or later. All changes: the yavs 0.16.0 release notes.

## 0.15.1 (2026-11-05)

- Needs yavs 0.15.1 or later. All changes: the yavs 0.15.1 release notes.

## 0.15.0 (2026-11-04)

- Needs yavs 0.15.0 or later. All changes: the yavs 0.15.0 release notes.

## 0.14.0 (2026-11-04)

- Needs yavs 0.14.0 or later. All changes: the yavs 0.14.0 release notes.

## 0.13.0 (2026-11-02)

- Needs yavs 0.13.0 or later. All changes: the yavs 0.13.0 release notes.

## 0.12.1 (2026-11-02)

- Needs yavs 0.12.1 or later. All changes: the yavs 0.12.1 release notes.

## 0.12.0 (2026-11-01)

- Vectors show on one line in the Variables view, Watch and hover; `help` in the Debug Console has command names in bold; Tab in the Debug Console takes what the list has in common and opens the list when none is open.
- Force and Release no longer ask for `in` or `out`; a value for a bit-like target needs no quotes (`1`, `0101`).
- Watch and the call stack show dotted names, and a Watch of a name that does not exist shows an error with the nearest name.
- "Show Drivers" in the context menu of a signal, and "Find Signals or Variables...": a quick pick of matches to add to the wave.
- The Breakpoints view has filters that stop at a report of a severity, with the stack intact.
- A launch that fails early shows why: the diagnostic in the Debug Console, or a notification with the command when yavs refuses its command line.
- The Debug Console completes commands, names, fields and source files, and shows the exit status at the end of a session.
- A Watch takes a field, an element, a slice, `.all` and expressions; the Variables view expands access values and lists the variables of a protected object.
- The problem matcher `$yavs` puts the diagnostics of a yavs task in the Problems view.
- The extension is released with yavs and carries its version (one release with every artifact).
- Needs yavs 0.12.0 or later. All changes: the yavs 0.12.0 release notes.

## 0.1.0

First release. The `yavs` debugger type (launch and attach over the Debug Adapter Protocol),
the `vhdl` language id for breakpoints, and the commands for running for or until a time,
stepping a delta or a time step, signal value breakpoints, and force, deposit and release.
Needs yavs 0.11.0 or newer.

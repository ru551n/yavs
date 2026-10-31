Open issues where valid VHDL gives an elaboration or lowering error (E2xxx, E3004, E9999) or the wrong diagnostic code. Most come from conformance suites; each links its failing test names.

## Issues
- [x] #909 lower: an aggregate with others and an impure composite expression runs others on every element first
- [x] #907 elab: an others aggregate in a constant or signal initial value calls an impure function once, not once per element (records)
- [x] #880 elab: port actual indexed by a signal is rejected (E2106) although the reference simulators run it
- [x] #875 elab: run-time packages are initialised before every elaboration-time call; the reference simulators elaborate a package when the first unit using it is elaborated
- [x] #874 elab: package initialisation reports are printed again when the packages are initialised a second time (coverage run, session rebuild)
- [x] #868 elab/cli: not-implemented errors of elaboration and commands still use E9999, the internal-error code
- [x] #848 elab: package subtype bounds of a package loaded during elaboration are evaluated again (stale range index)
- [x] #847 An unused package constant or architecture subtype with an index constraint outside its index subtype is not reported
- [x] #817 elab: a resolution function declared in a block is E9999 (VESTs tc3153, tc3156, tc3159)
- [x] #816 elab: individual associations of a record generic in a generic map are E9999 (VESTs tc1781)
- [x] #656 elab: a guarded (bus) port has no guard kind or disconnection time: a guarded assignment to it is E3004 at run time
- [x] #620 elab: now in a constant that bounds a port or signal subtype is E2106
- [x] #619 elab: 'instance_name and 'path_name of an instance label in a generic map are E2106
- [x] #618 elab: individually associated unconstrained record formals and aggregate targets of arrays of logic arrays are E2001
- [x] #615 lower: 'driving and 'driving_value of signal parameters and record parts are E2001
- [x] #610 lower: an aggregate signal target with parts of a signal parameter is E2001
- [x] #608 lower: a resolution function that is an alias of a predefined function is an internal error
- [x] #602 lower: null waveform elements in aggregate targets and signal parameters are still E2001

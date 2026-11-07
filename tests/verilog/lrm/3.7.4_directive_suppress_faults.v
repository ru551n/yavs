// LRM 3.7.4: a directive of fault simulation is not part of the standard and has no effect on a functional simulation (V17)
// units: m
// expect: W4016 5:1 directive ignored
`timescale 1ns/1ps
`suppress_faults
module m;
endmodule

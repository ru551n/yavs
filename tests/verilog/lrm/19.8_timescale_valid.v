// LRM 19.8: `timescale gives a unit and a precision, each 1, 10 or 100 of s, ms, us, ns, ps or fs
// units: m
// expect: none
`timescale 100 ps / 10 fs
module m;
endmodule

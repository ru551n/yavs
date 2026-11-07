// LRM 19.8: modules with and without a timescale in one design are an error (known gap G9: yavs warns, W4006)
// units: a,b
// gap: G9 the LRM error is a warning here
// expect: W4006 9:8
`timescale 1ns/1ps
module a;
endmodule
`resetall
module b;
endmodule

// LRM 19.2: only the net types that can be implicit are legal: supply0 is not
// expect: E4009 4:1 supply0 cannot be implicit
`timescale 1ns/1ps
`default_nettype supply0
module m;
endmodule

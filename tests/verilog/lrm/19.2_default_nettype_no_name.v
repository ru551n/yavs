// LRM 19.2: `default_nettype is followed by a net type
// expect: E4009 4:1 no net type
`timescale 1ns/1ps
`default_nettype
module m;
endmodule

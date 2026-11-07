// LRM 19.4: `else belongs to an open conditional
// expect: E4004 4:1 no open conditional
`timescale 1ns/1ps
`else
module a;
endmodule

// LRM 19.4: `endif closes an open conditional
// expect: E4004 6:1 no open conditional
`timescale 1ns/1ps
module a;
endmodule
`endif

// LRM 19.5: `include is followed by a quoted file name
// expect: E4009 4:1 no file name
`timescale 1ns/1ps
`include
module m;
endmodule

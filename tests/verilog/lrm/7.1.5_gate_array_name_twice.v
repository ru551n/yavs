// LRM 7.1.5: one instance identifier names one array of instances
// expect: E4203 6:41 array name again
`timescale 1ns/1ps
module m;
  wire [7:0] o, a, b;
  nand t[0:3] (o[3:0], a[3:0], b[3:0]), t[4:7] (o[7:4], a[7:4], b[7:4]);
endmodule

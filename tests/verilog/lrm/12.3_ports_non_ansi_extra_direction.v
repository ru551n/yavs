// LRM 12.3: a direction declaration names a port of the list
// expect: E4202 6:9 stranger is not in the port list
`timescale 1ns/1ps
module m (a);
  input a;
  input stranger;
endmodule

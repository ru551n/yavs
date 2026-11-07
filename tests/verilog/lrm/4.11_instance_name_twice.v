// LRM 4.11: the instances of a module scope have distinct names
// expect: E4203 8:5 instance name again
`timescale 1ns/1ps
module c;
endmodule
module m;
  c u ();
  c u ();
endmodule

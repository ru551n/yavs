// LRM 12.3.2: there is one port definition per port name
// expect: E4202 4:19 port name again
`timescale 1ns/1ps
module m (.a(x), .a(y));
  input x, y;
endmodule

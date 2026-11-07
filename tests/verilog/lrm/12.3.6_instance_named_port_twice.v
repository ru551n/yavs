// LRM 12.3.6: a port is connected once in a connection by name
// expect: E4100 7:16 port connected twice
module c (input i, output o);
endmodule
module m;
  wire a, b, x;
  c u (.i(a), .i(b), .o(x));
endmodule

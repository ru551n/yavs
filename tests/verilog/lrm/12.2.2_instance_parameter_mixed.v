// LRM 12.2.2: parameter assignments are all by order or all by name
// expect: E4100 7:10 ordered then named
module c #(parameter p = 1, q = 2) (input i);
endmodule
module m;
  wire a;
  c #(1, .q(3)) u (a);
endmodule

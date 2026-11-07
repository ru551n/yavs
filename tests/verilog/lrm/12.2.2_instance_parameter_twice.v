// LRM 12.2.2: a parameter is assigned once in an assignment by name
// expect: E4100 7:15 parameter assigned twice
module c #(parameter p = 1) (input i);
endmodule
module m;
  wire a;
  c #(.p(1), .p(2)) u (a);
endmodule

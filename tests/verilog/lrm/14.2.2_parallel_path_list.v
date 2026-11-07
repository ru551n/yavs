// LRM 14.2.2: a parallel path (=>) has one input and one output; lists need *>
// expect: E4100 5:17 list in a parallel path
module m (input a, b, output q);
  specify
    (a, b => q) = 1;
  endspecify
endmodule

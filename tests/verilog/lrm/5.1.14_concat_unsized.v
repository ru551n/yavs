// LRM 5.1.14: an unsized constant has no place in a concatenation
// expect: E4100 6:18 unsized constant
module m;
  reg [7:0] a;
  wire [8:0] w;
  assign w = {a, 1};
endmodule

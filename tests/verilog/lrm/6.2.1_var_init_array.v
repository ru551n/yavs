// LRM 6.2.1: a variable declaration assignment is not allowed on an array
// expect: E4100 4:13 array with initial value
module m;
  reg [3:0] mem [3:0] = 0;
endmodule

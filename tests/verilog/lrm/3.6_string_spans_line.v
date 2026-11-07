// LRM 3.6: a string may not contain a raw newline
// expect: E4014 5:20 newline inside a string
`timescale 1ns/1ps
module m;
  initial $display("one
  );
endmodule

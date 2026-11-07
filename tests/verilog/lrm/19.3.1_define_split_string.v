// LRM 19.3.1: macro text is not split across a string token: a text that opens a string and never closes it
// expect: E4014 5:20 unterminated string
`define first_half "start of string
module m;
  initial $display(`first_half);
endmodule

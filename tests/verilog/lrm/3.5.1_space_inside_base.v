// LRM 3.5.1: the apostrophe and the base letter are not separated by white space
// expect: E4003 4:18 space after apostrophe
module m;
  parameter p = 8' h FF;
endmodule

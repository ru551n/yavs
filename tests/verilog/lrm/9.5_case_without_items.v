// LRM 9.5: a case statement has at least one case item
// expect: E4100 6:3 no case item
module m;
  reg [1:0] s;
  initial case (s)
  endcase
endmodule

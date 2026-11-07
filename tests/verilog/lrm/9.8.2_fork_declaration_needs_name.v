// LRM 9.8.2: a parallel block has declarations only if it is named
// expect: E4100 5:5 declaration in unnamed fork
module m;
  initial fork
    integer i;
  join
endmodule

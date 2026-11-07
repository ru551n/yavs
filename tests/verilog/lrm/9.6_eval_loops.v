// LRM 9.6: for, while and repeat loops
// expect: none
// output: 10 4 3
`timescale 1ns/1ps
module m;
  integer i, s, n, k;
  initial begin
    s = 0;
    for (i = 0; i < 5; i = i + 1)
      s = s + i;
    n = 0;
    while (n < 4)
      n = n + 1;
    k = 0;
    repeat (3) k = k + 1;
    $display("%0d %0d %0d", s, n, k);
  end
endmodule

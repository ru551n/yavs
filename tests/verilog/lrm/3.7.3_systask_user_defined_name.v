// LRM 3.7.3: a name beginning with $ that the language does not define is a legal system task name (user PLI tasks); it is resolved after analysis
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  initial $my_pli_task(1, 2);
endmodule

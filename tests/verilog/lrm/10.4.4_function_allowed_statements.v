// LRM 10.4.4: a function may call system tasks, use blocking assignments, loops and conditions
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  function integer f;
    input integer x;
    integer i;
    begin
      f = 0;
      for (i = 0; i < x; i = i + 1)
        if (i > 1) f = f + i;
      $display("f=%0d", f);
    end
  endfunction
endmodule

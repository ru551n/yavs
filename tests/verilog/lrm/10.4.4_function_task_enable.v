// LRM 10.4.4: a function does not enable a task
// expect: E4100 10:7 task enable in a function
module m;
  task t;
    begin end
  endtask
  function f;
    input x;
    begin
      t;
      f = x;
    end
  endfunction
endmodule

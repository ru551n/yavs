// LRM 17.7: $time, $stime and $realtime return the simulation time
// units: m
// expect: none
`timescale 1ns/1ps
module m;
  time t;
  real r;
  initial begin
    t = $time;
    t = $stime;
    r = $realtime;
  end
endmodule

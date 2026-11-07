`timescale 1ns / 1ps
// Behavioural constructs: tasks, functions, loops, named blocks, disable, fork/join, wait, memories, real and time.
module tasks_functions;
    integer i, acc;
    time t0;
    realtime rt;
    real r;
    reg signed [15:0] sv;
    reg [7:0] mem [0:255];
    reg done;
    event ev;

    function integer clog2;
        input integer value;
        integer v;
        begin
            v = value - 1;
            for (clog2 = 0; v > 0; clog2 = clog2 + 1)
                v = v >> 1;
        end
    endfunction

    function automatic [7:0] pick(input [7:0] a, input [7:0] b, input sel);
        pick = sel ? a : b;
    endfunction

    task bump;
        inout integer x;
        input integer by;
        begin
            x = x + by;
        end
    endtask

    task automatic wait_done;
        begin
            wait (done);
            -> ev;
        end
    endtask

    initial begin : main
        acc = 0;
        done = 0;
        t0 = $time;
        r = 2.0 ** 3;
        sv = -16'sd5;
        for (i = 0; i < 4; i = i + 1) bump(acc, i);
        while (acc < 20) acc = acc + 1;
        repeat (3) #1 acc = acc - 1;
        fork
            #5 done = 1;
            begin : worker
                #2 acc = clog2(100);
                disable worker;
            end
        join
        mem[0] = pick(8'h01, 8'h02, acc[0]);
        forever begin
            @(ev) disable main;
        end
    end
endmodule

`timescale 1ns / 1ps
// System tasks and functions from the corpus survey; parsed here, executed later.
module system_tasks;
    integer fd, n, seed, val;
    reg [8*16-1:0] str;
    reg [31:0] word;
    reg [7:0] mem [0:15];
    real r;
    initial begin
        $display("t=%t b=%b h=%h d=%d s=%s m=%m", $time, 4'b1010, 8'hff, 12, "x");
        $write("no newline");
        $timeformat(-9, 3, " ns", 12);
        fd = $fopen("out.txt", "w");
        $fdisplay(fd, "line %0d", 1);
        $fwrite(fd, "w");
        $fflush(fd);
        $fclose(fd);
        fd = $fopen("in.txt", "r");
        n = $fscanf(fd, "%h", word);
        n = $fgetc(fd);
        n = $ungetc(n, fd);
        n = $fgets(str, fd);
        n = $sscanf("12 34", "%d", val);
        $sformat(str, "v=%0d", val);
        $readmemh("init.hex", mem);
        $readmemb("init.bin", mem, 0, 15);
        seed = 7;
        val = $random(seed) + $random + $dist_uniform(seed, 0, 9);
        if ($value$plusargs("N=%d", val)) val = $clog2(val);
        r = $itor(val) + $bitstoreal($realtobits(1.5)) + $realtime;
        val = $rtoi(r) + $signed(word[7:0]) + $stime;
        $stop;
        $finish;
    end
endmodule

Testbenches with clock generators (`clk <= not clk after 5 ns;`) simulate faster: the clock runs inside the kernel and an edge costs about half the instructions.

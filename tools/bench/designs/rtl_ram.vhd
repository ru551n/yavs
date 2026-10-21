-- A 16384 x 32-bit RAM as an array signal with one write and one read per cycle, 20000 cycles:
-- indexed signal assignment into a large array.
-- Head-to-head design (tools/bench/AGENTS.md); written for yavs.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity bench_rtl_ram is generic (DEPTH : natural := 16384; CYC : natural := 20000); end;
architecture a of bench_rtl_ram is
  signal clk : std_logic := '0';
  type mem_t is array (0 to DEPTH-1) of std_logic_vector(31 downto 0);
  signal mem : mem_t := (others => (others => '0'));
  signal waddr, raddr : unsigned(13 downto 0) := (others => '0');
  signal wdata, rdata : std_logic_vector(31 downto 0) := (others => '0');
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if rising_edge(clk) then
      mem(to_integer(waddr)) <= wdata;
      rdata <= mem(to_integer(raddr));
    end if;
  end process;
  process
    variable acc : std_logic_vector(31 downto 0) := (others => '0');
  begin
    for c in 1 to CYC loop
      wait until rising_edge(clk);
      waddr <= waddr + 1; raddr <= waddr - 3;
      wdata <= std_logic_vector(unsigned(wdata) + 12345);
      acc := acc xor rdata;
    end loop;
    report "acc=" & to_hstring(acc);
    done <= true; wait;
  end process;
end;

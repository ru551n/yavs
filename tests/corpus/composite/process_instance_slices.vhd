-- difftest: top=tb_process_instance_slices std=2008
-- Slices whose bounds come from generics and generate parameters, in clocked and combinational
-- processes and in the sensitivity of a process: the positions, descriptors and the null slice
-- (a range with no element, which checks nothing) are the same at every activation.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cell is
  generic (HI : natural; LO : natural; NUL : integer; ID : natural);
  port (clk : in bit; v : in std_logic_vector(7 downto 0); o : out std_logic_vector(HI downto LO);
        z : out std_logic_vector(NUL downto 2); w : out integer);
end;
architecture a of cell is
  signal c : natural := 0;
begin
  process (clk) begin
    if clk = '1' then
      o <= v(HI downto LO) xor ('0' & v(HI downto LO + 1));
      c <= c + 1;
    end if;
  end process;
  -- sensitive to a slice with generic bounds
  process (v(HI downto LO)) begin
    w <= to_integer(unsigned(v(HI downto LO))) + ID;
  end process;
  -- a null slice of the input, whatever the direction of its range
  z <= v(NUL downto 2);
end;

library ieee;
use ieee.std_logic_1164.all;
entity tb_process_instance_slices is end;
architecture t of tb_process_instance_slices is
  signal clk : bit := '0';
  signal v : std_logic_vector(7 downto 0) := "10110100";
  signal o1 : std_logic_vector(5 downto 2);
  signal o2 : std_logic_vector(7 downto 4);
  signal z1 : std_logic_vector(1 downto 2);
  signal z2 : std_logic_vector(3 downto 2);
  signal w1, w2 : integer;
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  u1 : entity work.cell generic map (5, 2, 1, 10) port map (clk, v, o1, z1, w1);
  u2 : entity work.cell generic map (7, 4, 3, 20) port map (clk, v, o2, z2, w2);
  process
  begin
    wait for 12 ns;
    assert o1 = "1011" report "o1 " & to_string(o1) severity failure;
    assert o2 = "1110" report "o2 " & to_string(o2) severity failure;
    assert z1'length = 0 and z2 = "01" report "z" severity failure;
    assert w1 = 23 and w2 = 31 report "w " & integer'image(w1) & " " & integer'image(w2) severity failure;
    v <= "01101011";
    wait for 10 ns;
    assert o1 = "1111" report "o1b " & to_string(o1) severity failure;
    assert o2 = "0101" report "o2b " & to_string(o2) severity failure;
    assert w1 = 20 and w2 = 26 report "wb " & integer'image(w1) & " " & integer'image(w2) severity failure;
    done <= true;
    report "ok";
    wait;
  end process;
end;

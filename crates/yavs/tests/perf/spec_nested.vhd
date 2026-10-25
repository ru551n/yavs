-- Processes with subprograms nested in them, some also holding shared state (an access object and
-- a line): a specialised copy keeps the frame the nested subprograms reach, and folds the
-- layouts of the variables they do not write. The checksum must not depend on the copy.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
use std.textio.all;
entity spec_nest is
  generic (W : positive := 8; K : natural := 1; LOG : boolean := false);
  port (clk : in std_logic; d : in std_logic_vector(W - 1 downto 0);
        q : out std_logic_vector(W - 1 downto 0); n : out natural);
end;
architecture a of spec_nest is
begin
  process (clk)
    type ptr is access bit_vector;
    variable p : ptr;
    variable v : bit_vector(W - 1 downto 0) := (others => '0');
    variable wd : bit_vector(2 * W - 1 downto 0) := (others => '1');
    variable cnt : natural := 0;
    variable l : line;
    procedure step(x : bit_vector) is
    begin
      cnt := (cnt + 1) mod 1000;
      v := v(W - 2 downto 0) & x(x'left);
      wd(W + K downto K) := v & '1';
    end procedure;
    impure function ones return natural is
      variable c : natural := 0;
    begin
      for i in v'range loop
        if v(i) = '1' then c := c + 1; end if;
      end loop;
      return c + wd'length;
    end function;
  begin
    if rising_edge(clk) then
      if p = null then p := new bit_vector(W - 1 downto 0); end if;
      step(to_bitvector(d));
      p.all := p.all xor v;
      if LOG and cnt = 100000 then
        write(l, ones);
        writeline(output, l);
      end if;
      q <= to_stdlogicvector(p.all xor wd(W - 1 downto 0));
      n <= (ones + cnt) mod 1000;
    end if;
  end process;
end;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity spec_nested is end;
architecture a of spec_nested is
  signal clk : std_logic := '0';
  signal done : boolean := false;
  signal d8 : std_logic_vector(7 downto 0) := x"A5";
  signal d12 : std_logic_vector(11 downto 0) := x"3C7";
  signal d6 : std_logic_vector(5 downto 0) := "101101";
  signal q8, q12, q6 : std_logic_vector(11 downto 0);
  signal q8a : std_logic_vector(7 downto 0);
  signal q12a : std_logic_vector(11 downto 0);
  signal q6a : std_logic_vector(5 downto 0);
  signal n8, n12, n6 : natural;
  signal cyc : natural := 0;
begin
  clk <= not clk after 5 ns when not done;
  a8: entity work.spec_nest generic map (8, 1, true) port map (clk, d8, q8a, n8);
  c12: entity work.spec_nest generic map (12, 3) port map (clk, d12, q12a, n12);
  c6: entity work.spec_nest generic map (6, 2) port map (clk, d6, q6a, n6);
  process (clk)
  begin
    if rising_edge(clk) then
      cyc <= cyc + 1;
      d8 <= std_logic_vector(unsigned(d8) + 3);
      d12 <= std_logic_vector(unsigned(d12) + 5);
      d6 <= std_logic_vector(unsigned(d6) + 1);
      if cyc = 300 then
        report "sum=" & integer'image(to_integer(unsigned(q8a))) & " " & integer'image(to_integer(unsigned(q12a)))
          & " " & integer'image(to_integer(unsigned(q6a))) & " " & integer'image(n8) & " " & integer'image(n12) & " " & integer'image(n6);
        done <= true;
      end if;
    end if;
  end process;
end;

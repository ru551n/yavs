-- Processes specialised to their instance's data: four instances of one entity with three distinct
-- generic sets (two share a copy), and a generate whose iterations share one. The checksum must
-- not depend on whether the processes run the shared code or their copies.
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity spec_acc is
  generic (W : positive := 8; K : natural := 1);
  port (clk : in std_logic; d : in std_logic_vector(W - 1 downto 0);
        q : out std_logic_vector(W - 1 downto 0));
end;
architecture a of spec_acc is
  signal r : std_logic_vector(W - 1 downto 0) := (others => '0');
begin
  process (clk) begin
    if rising_edge(clk) then
      r <= (r(W - 2 downto 0) & d(W - 1)) xor (d(K downto 0) & r(W - 1 downto K + 1));
    end if;
  end process;
  q <= r;
end;

library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity spec_copies is end;
architecture a of spec_copies is
  signal clk : std_logic := '0';
  signal done : boolean := false;
  signal d8 : std_logic_vector(7 downto 0) := x"A5";
  signal d12 : std_logic_vector(11 downto 0) := x"3C7";
  signal d6 : std_logic_vector(5 downto 0) := "101101";
  signal q8a, q8b : std_logic_vector(7 downto 0);
  signal q12 : std_logic_vector(11 downto 0);
  type v6 is array (0 to 2) of std_logic_vector(5 downto 0);
  signal q6 : v6;
begin
  clk <= not clk after 5 ns when not done;
  a8: entity work.spec_acc generic map (8, 1) port map (clk, d8, q8a);
  b8: entity work.spec_acc generic map (8, 1) port map (clk, d8, q8b);
  c12: entity work.spec_acc generic map (12, 3) port map (clk, d12, q12);
  g: for i in 0 to 2 generate
    u: entity work.spec_acc generic map (6, 2) port map (clk, d6, q6(i));
  end generate;
  process (clk) begin
    if rising_edge(clk) then
      d8 <= d8(6 downto 0) & d8(7);
      d12 <= d12(10 downto 0) & (d12(11) xor d12(3));
      d6 <= d6(4 downto 0) & d6(5);
    end if;
  end process;
  process
    variable acc : unsigned(31 downto 0) := (others => '0');
  begin
    wait for 1500 ns;
    acc := acc + unsigned(q8a) + unsigned(q8b) * 3 + unsigned(q12) * 7;
    for i in q6'range loop
      acc := acc + unsigned(q6(i)) * (i + 11);
    end loop;
    report "sum=" & to_hstring(std_logic_vector(acc));
    done <= true; wait;
  end process;
end;

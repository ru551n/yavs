-- A child instance whose out port is the only source of the parent's signal (collapsed): the
-- port needs no edge in the kernel, the signal updates in the usual one-signal delta.
library ieee; use ieee.std_logic_1164.all;
entity child is port (clk : in std_logic; q : out std_logic_vector(7 downto 0)); end;
architecture a of child is
  signal r : std_logic_vector(7 downto 0) := x"01";
begin
  process (clk) begin
    if rising_edge(clk) then r <= r(6 downto 0) & r(7); end if;
  end process;
  q <= r;
end;
library ieee; use ieee.std_logic_1164.all;
entity out_port is end;
architecture a of out_port is
  signal clk : std_logic := '0';
  signal q : std_logic_vector(7 downto 0);
  signal done : boolean := false;
begin
  clk <= not clk after 5 ns when not done;
  u: entity work.child port map (clk => clk, q => q);
  process begin
    wait for 500 ns;
    report "q=" & to_hstring(q);
    done <= true; wait;
  end process;
end;

-- difftest: top=copy_resume_waits stop=15002ns
-- Monitors of generate lanes on elements of an integer array signal, one with a timed wait and one
-- whose element index is computed from the parameter, reading 'event, 'active and 'quiet (#1138).
-- A process switches to its specialised copy between two activations (after 1024 of them by
-- default); the first activation of the copy must see this lane's element. The first and the last
-- element never change. Each monitor runs about 1500 activations, past the switch.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity copy_resume_waits is
  generic (N : positive := 3);
end entity;
architecture t of copy_resume_waits is
  type int_array_t is array (natural range <>) of integer;
  signal clk  : std_logic := '0';
  signal done : boolean := false;
  signal cnt  : unsigned(7 downto 0) := (others => '0');
  signal iv   : int_array_t(0 to N + 1) := (others => 0);
  signal n2, n3 : int_array_t(1 to N) := (others => 0);
begin
  clk <= not clk after 5 ns when not done;
  process (clk) begin
    if rising_edge(clk) then cnt <= cnt + 1; end if;
  end process;
  g : for i in 0 to N - 1 generate
    process (clk) begin
      if rising_edge(clk) then
        iv(i + 1) <= to_integer(cnt) + i;
      end if;
    end process;
    process
      variable k : natural := 0;
    begin
      wait until iv(i + 1)'event for 20 ns;
      assert iv(i + 1)'event report "int element" severity failure;
      k := k + 1;
      n2(i + 1) <= k;
    end process;
    process
      variable k : natural := 0;
    begin
      wait on iv((i + 2) mod N + 1);
      assert iv((i + 2) mod N + 1)'active and not iv((i + 2) mod N + 1)'quiet report "quiet int" severity failure;
      k := k + 1;
      n3(i + 1) <= k;
    end process;
  end generate;
  process begin
    wait for 15 us;
    done <= true;
    wait for 1 ns;
    for i in 1 to N loop
      report "lane " & integer'image(i) & ": " & integer'image(n2(i)) & " " & integer'image(n3(i));
    end loop;
    wait;
  end process;
end architecture;

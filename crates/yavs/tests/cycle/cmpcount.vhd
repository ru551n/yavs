-- Cycle mode gate design: a counter with an enable feeding comparators (combinational) that a
-- testbench process reads at every rising edge, with the threshold and the enable changed between
-- edges and at falling edges. The counter wraps and is loaded; the monitor prints all of it.
library ieee;
use ieee.std_logic_1164.all;
use ieee.numeric_std.all;
entity cycle_cmpcount is
end entity;
architecture a of cycle_cmpcount is
  signal clk, en, load : std_ulogic := '0';
  signal count : unsigned(5 downto 0) := (others => '0');
  signal ld_val, thr : unsigned(5 downto 0) := (others => '0');
  signal ge, eq, lt, wrapped : std_ulogic := '0';
  signal wraps : unsigned(3 downto 0) := (others => '0');
  impure function snap return string is
  begin
    return "count " & to_string(to_integer(count)) & " thr " & to_string(to_integer(thr)) & " en " & to_string(en)
      & " ge " & to_string(ge) & " eq " & to_string(eq) & " lt " & to_string(lt) & " wrapped " & to_string(wrapped)
      & " wraps " & to_string(to_integer(wraps));
  end function;
begin
  clk <= not clk after 5 ns;

  counter : process (clk)
  begin
    if rising_edge(clk) then
      wrapped <= '0';
      if load = '1' then
        count <= ld_val;
      elsif en = '1' then
        if count = 40 then
          count <= (others => '0');
          wrapped <= '1';
          wraps <= wraps + 1;
        else
          count <= count + 1;
        end if;
      end if;
    end if;
  end process;

  ge <= '1' when count >= thr else '0';
  eq <= '1' when count = thr else '0';
  lt <= not ge;

  -- two waits: stays outside the region; reads the values before the edge, then after it
  monitor : process
  begin
    wait until rising_edge(clk);
    report "pre  " & snap;
    wait for 1 ns;
    report "post " & snap;
  end process;

  stim : process
  begin
    for i in 0 to 119 loop
      wait until falling_edge(clk);
      en <= '0' when i mod 5 = 4 else '1';
      load <= '1' when i = 60 or i = 61 else '0';
      ld_val <= to_unsigned(35 + i mod 3, 6);
      if i mod 9 = 0 then
        thr <= to_unsigned((i * 3) mod 45, 6);
      end if;
    end loop;
    wait for 1 ns;
    std.env.stop;
  end process;
end architecture;

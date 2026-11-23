-- A clocked member keeping a register file of more than 64 words without --two-state:
-- `en` is 'U', so the normal mode shifts nothing and every word keeps its start value; the
-- second image reads `en` as 1 and shifts all 80 words, which `rail_keep` must give back their
-- values (each word starts with its own; word `I` is shown).
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity cycle_railkeep is
  generic (I : natural := 70);
end;
architecture a of cycle_railkeep is
  signal clk : std_ulogic := '0';
  signal en : std_ulogic;
  type file_t is array (0 to 79) of std_ulogic_vector(63 downto 0);
  function init return file_t is
    variable r : file_t;
  begin
    for k in r'range loop
      r(k) := std_ulogic_vector(to_unsigned(k * 4099 + 17, 64));
    end loop;
    return r;
  end function;
  signal rf : file_t := init;
  signal d : std_ulogic_vector(63 downto 0) := (others => '0');
begin
  clk <= not clk after 5 ns when now < 100 ns else clk;
  wr : process (clk) begin
    if rising_edge(clk) then
      if en = '1' then
        rf(0 to 78) <= rf(1 to 79);
        rf(79) <= d;
      end if;
    end if;
  end process;
  stim : process begin
    wait for 22 ns;
    report "rf=" & to_hstring(rf(I)) & " rf0=" & to_hstring(rf(0));
    wait;
  end process;
end;

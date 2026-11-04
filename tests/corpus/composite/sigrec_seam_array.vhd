-- difftest: top=sigrec_seam_array
-- An array of small records driven whole, one leaf of one element changing while the other leaf
-- holds (#1050): the bytes of one leaf lie between those of the other, and what a scatter keeps of
-- one must not hide a change of the other.
library ieee;
use ieee.std_logic_1164.all;

entity sigrec_seam_array is
end entity;

architecture t of sigrec_seam_array is
  type rec_t is record
    a : std_ulogic;
    b : std_ulogic;
  end record;
  type arr_t is array (0 to 39) of rec_t;
  signal s : arr_t := (others => ('0', '0'));
  signal clk : bit := '0';
  signal step : integer := 0;
begin
  clk <= not clk after 5 ns when step < 60;

  drive : process (clk)
    variable v : arr_t := (others => ('0', '0'));
  begin
    if clk = '1' then
      step <= step + 1;
      if step mod 5 = 0 then v(step mod 40).b := not v(step mod 40).b; end if;
      if step mod 7 = 0 then v(39 - step mod 40).a := not v(39 - step mod 40).a; end if;
      s <= v;
    end if;
  end process;

  show : process (clk)
    variable n : integer;
  begin
    if clk = '0' then
      n := 0;
      for i in s'range loop
        n := n * 3 mod 65521;
        if s(i).a = '1' then n := n + 1; end if;
        n := n * 3 mod 65521;
        if s(i).b = '1' then n := n + 1; end if;
      end loop;
      report integer'image(step) & " s=" & integer'image(n);
    end if;
  end process;
end architecture;

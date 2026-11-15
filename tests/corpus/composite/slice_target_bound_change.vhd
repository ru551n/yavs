-- difftest: top=slice_target_bound_change std=2008
-- A slice target is checked before its value (#1156); a call in the value that moves the slice's
-- bound changes neither the slice written nor its length check (sim1 and sim2 agree).
library ieee; use ieee.std_logic_1164.all; use ieee.numeric_std.all;
entity slice_target_bound_change is end;
architecture a of slice_target_bound_change is
begin
  process
    variable n : natural := 7;
    variable v : std_ulogic_vector(23 downto 0) := (others => '0');
    impure function f return std_ulogic_vector is begin n := n + 8; return std_ulogic_vector(to_unsigned(n, 8)); end function;
  begin
    v(n downto n - 7) := f;
    report "RES " & to_string(v) & " " & integer'image(n);
    v(n downto n - 7) := std_ulogic_vector(to_unsigned(5, 8)) or f;
    report "RES " & to_string(v) & " " & integer'image(n);
    wait;
  end process;
end;

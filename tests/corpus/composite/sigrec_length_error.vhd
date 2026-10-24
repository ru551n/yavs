-- difftest: top=sigrec_length_error std=2008 exit=1
-- A record whose unconstrained field has another length than the record signal's is a length
-- error (LRM 10.5.2.1), also when both lengths fit the same plane block (#677).
library ieee; use ieee.std_logic_1164.all;
entity sigrec_length_error is end;
architecture a of sigrec_length_error is
  type rec_t is record
    a : std_logic_vector;
    b : integer;
  end record;
  subtype r8 is rec_t(a(7 downto 0));
  subtype r4 is rec_t(a(3 downto 0));
  signal s : r8;
begin
  p : process
    variable ok : r8 := (a => x"5a", b => 1);
    variable v : r4 := (a => "0101", b => 3);
  begin
    -- Equal lengths with other bounds are fine.
    s <= (a => "01011010", b => 2);
    wait for 1 ns;
    report "before: " & to_string(s.a) & " " & integer'image(s.b);
    s <= ok;
    wait for 1 ns;
    report "same: " & to_string(s.a) & " " & integer'image(s.b);
    s <= v;
    wait for 1 ns;
    report "after: " & to_string(s.a) & " " & integer'image(s.b);
    wait;
  end process;
end architecture;

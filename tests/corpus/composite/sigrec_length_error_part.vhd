-- difftest: top=sigrec_length_error_part std=2008 exit=1
-- The length check of a record value driven onto an element of an array of records, and onto a
-- field that is a record (#677).
library ieee; use ieee.std_logic_1164.all;
entity sigrec_length_error_part is end;
architecture a of sigrec_length_error_part is
  type rec_t is record
    a : std_logic_vector;
    b : integer;
  end record;
  subtype r8 is rec_t(a(7 downto 0));
  subtype r4 is rec_t(a(3 downto 0));
  type cells is array (natural range <>) of r8;
  signal sc : cells(0 to 2);
begin
  p : process
    variable ok : r8 := (a => x"5a", b => 1);
    variable w : r4 := (a => "0101", b => 3);
  begin
    for i in sc'range loop
      sc(i) <= ok;
    end loop;
    wait for 1 ns;
    report "ok: " & to_string(sc(2).a) & " " & integer'image(sc(1).b);
    sc(1) <= w;
    wait for 1 ns;
    report "not reached";
    wait;
  end process;
end architecture;

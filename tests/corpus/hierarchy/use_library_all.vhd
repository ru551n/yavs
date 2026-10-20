-- difftest: top=tb_use_library_all std=2008
-- `use work.all` makes the units of the library visible; names that are no unit (an operator,
-- a literal) are looked up as usual.
package pk_ula is
  constant v : bit_vector := "10";
  type t_ula is (red, green);
end package;

entity tb_use_library_all is end;
use work.all;
architecture t of tb_use_library_all is
begin
  process
  begin
    assert pk_ula.v = "10";
    report "ok";
    wait;
  end process;
end;

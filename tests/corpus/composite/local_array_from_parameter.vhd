-- difftest: top=local_array_from_parameter std=2008
-- Local arrays whose bounds come from a parameter (#771): their descriptor is made in place,
-- their default values (logic, bit, character and integer elements) fill the object, a
-- null range is fine, and the local is checked against the parameter's bounds at the end.
library ieee; use ieee.std_logic_1164.all;
entity local_array_from_parameter is end;
architecture a of local_array_from_parameter is
  type int_vec is array (natural range <>) of integer range -5 to 5;
  procedure fill(value : out std_ulogic_vector; ok : out boolean) is
    constant sl : integer := value'length;
    variable sv : std_ulogic_vector(0 to sl - 1);
    variable bv : bit_vector(1 to sl);
    variable s : string(1 to sl / 4 - 1);
    variable iv : int_vec(0 to sl - 1);
  begin
    ok := false;
    if sl > 0 then
      assert sv(0) = 'U' and bv(1) = '0' and iv(sl - 1) = -5 report "defaults" severity failure;
    end if;
    assert s'length = sl / 4 - 1 or (sl < 4 and s'length = 0) report "string length" severity failure;
    for i in sv'range loop
      sv(i) := '1';
    end loop;
    for i in s'range loop
      assert s(i) = character'val(0) report "string default" severity failure;
    end loop;
    ok := true;
    value := sv;
  end procedure;
begin
  process
    variable v0 : std_ulogic_vector(1 downto 0 + 1);
    variable v8 : std_ulogic_vector(7 downto 0);
    variable v12 : std_ulogic_vector(20 to 31);
    variable ok : boolean;
  begin
    fill(v0, ok);
    assert ok report "null" severity failure;
    fill(v8, ok);
    assert ok and v8 = "11111111" report "v8" severity failure;
    fill(v12, ok);
    assert ok and v12 = "111111111111" and v12'left = 20 report "v12" severity failure;
    fill(v8(5 downto 2), ok);
    assert ok report "slice" severity failure;
    report "done";
    wait;
  end process;
end;

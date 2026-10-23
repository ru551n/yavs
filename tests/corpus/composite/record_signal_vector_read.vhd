-- difftest: top=record_signal_vector_read std=2008
-- A whole record signal read into a variable, field by field: logic vectors of one, seven, 64, 65
-- and 130 elements (ascending and descending, all nine values, one and two plane blocks) in a
-- record and in a nested record, with a scalar field between them, and the same through
-- 'last_value, before and after the signal changes.
library ieee;
use ieee.std_logic_1164.all;

entity record_signal_vector_read is
end entity;

architecture a of record_signal_vector_read is
  type inner_t is record
    flag : std_ulogic;
    w65  : std_ulogic_vector(64 downto 0);
  end record;
  type rec_t is record
    one : std_ulogic_vector(0 downto 0);
    n   : integer;
    v7  : std_ulogic_vector(1 to 7);
    sub : inner_t;
    v64 : std_ulogic_vector(63 downto 0);
    wid : std_ulogic_vector(0 to 129);
  end record;

  function pattern(n : natural; salt : natural) return std_ulogic_vector is
    constant all9 : string := "UX01ZWLH-";
    variable r : std_ulogic_vector(0 to n - 1);
  begin
    for i in r'range loop
      r(i) := std_ulogic'value("'" & all9((i * 7 + salt) mod 9 + 1) & "'");
    end loop;
    return r;
  end function;

  function make(salt : natural) return rec_t is
    variable r : rec_t;
  begin
    r.one := pattern(1, salt);
    r.n := salt;
    r.v7 := pattern(7, salt);
    r.sub.flag := pattern(1, salt + 3)(0);
    r.sub.w65 := pattern(65, salt + 1);
    r.v64 := pattern(64, salt + 2);
    r.wid := pattern(130, salt + 4);
    return r;
  end function;

  signal s    : rec_t := make(0);
  signal copy : rec_t;
begin
  process
    variable v : rec_t;
  begin
    wait for 1 ns;
    v := s;
    assert v = make(0) report "first read" severity failure;
    assert v.sub.w65 = pattern(65, 1) and v.wid = pattern(130, 4) report "fields" severity failure;
    copy <= v;
    wait for 1 ns;
    assert copy = make(0) report "copy" severity failure;
    s <= make(5);
    wait for 1 ns;
    v := s;
    assert v = make(5) report "second read" severity failure;
    assert s'last_value = make(0) report "last_value" severity failure;
    -- a record read again after only a part of it changed
    s.v64 <= pattern(64, 7);
    wait for 1 ns;
    v := s;
    assert v.v64 = pattern(64, 7) and v.wid = pattern(130, 9) and v.n = 5
      report "part" severity failure;
    report "done";
    wait;
  end process;
end architecture;

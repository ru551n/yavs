-- difftest: top=guarded_explicit_guard std=2008
-- A guarded assignment in a block without a guard expression reads the signal named GUARD that
-- is visible there (LRM 11.6): here one declared in the architecture.
entity guarded_explicit_guard is end entity;

architecture a of guarded_explicit_guard is
  function rf (s : bit_vector) return bit is
  begin
    for i in s'range loop
      if s(i) = '1' then
        return '1';
      end if;
    end loop;
    return '0';
  end function;
  subtype rbit is rf bit;
  signal sig : rbit bus;
  signal guard : boolean := false;
  disconnect sig : rbit after 0 ns;
begin
  l1 : block
  begin
    sig <= guarded '1';
  end block;

  process
  begin
    wait for 1 ns;
    assert sig = '0' report "disconnected before GUARD is true" severity failure;
    guard <= true;
    wait for 1 ns;
    assert sig = '1' report "driven while GUARD is true" severity failure;
    guard <= false;
    wait for 1 ns;
    assert sig = '0' report "disconnected again" severity failure;
    report "done";
    wait;
  end process;
end architecture;

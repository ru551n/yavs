-- difftest: top=failure_exit std=2008 exit=1
entity failure_exit is end;
architecture t of failure_exit is
begin
  process begin
    report "before";
    wait for 3 ns;
    assert false report "fatal condition" severity failure;
    report "unreachable";
    wait;
  end process;
  process begin wait for 10 ns; report "other process must not run"; wait; end process;
end;

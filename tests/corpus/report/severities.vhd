-- difftest: top=severities std=2008 exit=1
entity severities is end;
architecture t of severities is
begin
  process begin
    report "default is note";
    report "note" severity note;
    report "warning" severity warning;
    report "error" severity error;
    assert false report "assert note" severity note;
    assert false report "assert warning" severity warning;
    assert false report "assert error" severity error;
    assert true report "never printed" severity error;
    assert false severity warning;
    assert false;
    wait for 1 ns;
    report "continues after error at " & time'image(now);
    wait;
  end process;
end;

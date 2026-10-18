-- difftest: top=error_then_end std=2008 exit=1
entity error_then_end is end;
architecture t of error_then_end is
begin
  process begin
    assert false report "an error, simulation still ends normally" severity error;
    wait;
  end process;
end;

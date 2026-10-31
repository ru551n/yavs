-- lint_770: a process that can never suspend.
entity suspend is
end entity;

architecture rtl of suspend is
  signal x : bit;
  procedure tick is
  begin
    null;
  end procedure;
begin
  named : process -- expect: lint_770
  begin
    report "again";
  end process named;

  process -- expect: lint_770
  begin
    report "again";
  end process;

  nested : process -- expect: lint_770
    variable i : integer := 0;
  begin
    if i > 3 then
      i := 0;
    end if;
  end process;

  listed : process (x) is
  begin
    report "changed";
  end process;

  all_of_it : process (all) is
  begin
    report "changed" & bit'image(x);
  end process;

  waits : process
  begin
    wait for 10 ns;
  end process;

  -- A wait that looks unreachable still counts.
  dead_wait : process
  begin
    if false then
      wait;
    end if;
  end process;

  -- The procedure may hold the wait.
  calls : process
  begin
    tick;
  end process;

  deep_wait : process
  begin
    for i in 0 to 1 loop
      case i is
        when 0 => wait for 1 ns;
        when others => null;
      end case;
    end loop;
  end process;
end architecture;

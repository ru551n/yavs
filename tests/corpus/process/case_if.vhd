-- difftest: top=case_if std=2008
entity case_if is end;
architecture t of case_if is
  type op is (add, sub, mul, nop);
  function f(o : op; a, b : integer) return integer is begin
    case o is
      when add => return a + b;
      when sub => return a - b;
      when mul => return a * b;
      when others => return 0;
    end case;
  end;
begin
  process begin
    for o in op loop
      report op'image(o) & " -> " & integer'image(f(o, 6, 3));
    end loop;
    for i in 0 to 5 loop
      case i is
        when 0 | 1 => report "low";
        when 2 to 3 => report "mid";
        when others => report "high " & integer'image(i);
      end case;
    end loop;
    wait;
  end process;
end;

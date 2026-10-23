-- Variables of enclosing subprograms (address up the static chain) and real compares: ops of the
-- fast loop, no generic steps.
entity static_chain is end;
architecture a of static_chain is
begin
  process
    variable acc : integer := 0;
    variable x : real := 0.5;
    procedure bump(n : integer) is
      variable t : integer := 0;
      procedure inner is
      begin
        t := t + n;
        acc := acc + n;
      end procedure;
    begin
      inner;
      inner;
      t := t + acc;
      acc := t mod 1000;
    end procedure;
  begin
    for rep in 1 to 25 loop
      for i in 1 to 4 loop
        bump(i);
        if x < 2.0 then x := x * 2.0; end if;
        if x /= 4.0 and x >= 1.0 then acc := acc + 100; end if;
      end loop;
    end loop;
    report "acc=" & integer'image(acc) & " x=" & real'image(x);
    wait;
  end process;
end;

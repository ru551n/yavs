-- LRM 9.2.2: `f and K` with K a constant false: the left operand is evaluated first, so its call (a report) happens.
entity top is end entity;
architecture a of top is
  constant K : boolean := false;
  impure function noisy return boolean is
  begin
    report "RES noisy";
    return true;
  end function;
begin
  process
  begin
    if noisy and K then
      report "RES taken";
    end if;
    report "RES end";
    wait;
  end process;
end architecture;

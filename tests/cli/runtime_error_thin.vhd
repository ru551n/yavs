-- A runtime error three thin subprograms deep: each only passes its argument on.
entity runtime_error_thin is end;
architecture t of runtime_error_thin is
  type counter_t is protected
    impure function divide(x : integer) return integer;
  end protected;
  type counter_t is protected body
    variable n : integer := 10;
    impure function divide(x : integer) return integer is
    begin
      return n / x;
    end;
  end protected body;
  shared variable counter : counter_t;

  impure function pass1(x : integer) return integer is
  begin
    return counter.divide(x);
  end;
  impure function pass2(x : integer) return integer is
  begin
    return pass1(x);
  end;
  impure function pass3(x : integer) return integer is
  begin
    return pass2(x);
  end;
begin
  process
    variable v : integer;
  begin
    v := pass3(5);
    wait for 2 ns;
    v := pass3(0);
    wait;
  end process;
end;

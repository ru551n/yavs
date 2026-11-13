-- Allocations in compiled code, on a native frame (the function) and in frame mode (the process
-- body): each `new` is one call of the host's allocation, not a step of the host.
entity new_compiled is end;
architecture a of new_compiled is
  type node;
  type node_ptr is access node;
  type node is record
    v    : integer;
    link : node_ptr;
  end record;
  type iv_ptr is access integer_vector;
  type str_ptr is access string;
  function churn(k, n : integer) return integer is
    variable h : node_ptr;
    variable iv : iv_ptr;
    variable s : str_ptr;
    variable t : integer := k;
  begin
    for i in 1 to n loop
      h := new node'(i, h);
      iv := new integer_vector'(i, k, 3);
      s := new string'("ab");
      t := t + h.v + iv(1) + s'length;
      deallocate(iv);
      deallocate(s);
    end loop;
    return t + h.link.v;
  end function;
begin
  process
    variable s : integer := 0;
    variable h : node_ptr;
    variable iv : iv_ptr;
  begin
    for k in 0 to 99 loop
      s := (s * 31 + churn(k, 4)) mod 99991;
      h := new node'(k, h);
      iv := new integer_vector'(k, s, 7, 8);
      s := (s + h.v + iv(2)) mod 99991;
      deallocate(iv);
      wait for 1 ns;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end;

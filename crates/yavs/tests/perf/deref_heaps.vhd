-- A protected object in the shared heap and one in a process heap, each with an access value
-- inside, of the same heap indices: every `Deref` of them after the first finds its object in
-- the table inline (the entry the runtime gives it, by the handle's tag byte).
entity deref_heaps is end;
architecture a of deref_heaps is
  type store is protected
    procedure init(n : integer);
    impure function get(i : integer) return integer;
  end protected;
  type store is protected body
    type iv_ptr is access integer_vector;
    variable p : iv_ptr;
    procedure init(n : integer) is
    begin
      p := new integer_vector(0 to n - 1);
      for i in 0 to n - 1 loop
        p(i) := i * 3;
      end loop;
    end procedure;
    impure function get(i : integer) return integer is
    begin
      return p(i);
    end function;
  end protected body;
  shared variable sh : store;
begin
  process
    variable pv : store;
    variable s : integer := 0;
  begin
    sh.init(16);
    pv.init(16);
    for k in 0 to 399 loop
      s := (s * 7 + sh.get(k mod 16) + pv.get((k * 5) mod 16)) mod 99991;
    end loop;
    report "s=" & integer'image(s);
    wait;
  end process;
end;

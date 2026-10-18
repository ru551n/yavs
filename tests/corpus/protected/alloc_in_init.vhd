-- difftest: top=c std=2008
entity c is end;
architecture t of c is
  type iv_ptr is access integer_vector;
  type prot is protected
    procedure push(x : integer); impure function n return natural;
  end protected;
  type prot is protected body
    variable p : iv_ptr := new integer_vector(0 to 3);
    variable k : natural := 0;
    procedure push(x : integer) is begin p(k) := x; k := k + 1; end;
    impure function n return natural is begin return k; end;
  end protected body;
  shared variable sv : prot;
begin
  process begin
    sv.push(3); sv.push(4);
    report "n=" & integer'image(sv.n);
    wait;
  end process;
end;

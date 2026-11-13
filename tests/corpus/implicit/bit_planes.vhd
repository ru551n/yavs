-- difftest: top=bit_planes std=2008
-- The predefined operations of bit_vector and boolean_vector that work on the bit planes a word
-- at a time, against plain element loops: sll, srl, rol, ror (counts of both signs, zero, beyond
-- the length), sla and sra (counts from 0 up: negative ones are in hand_edges), the reductions
-- (LRM 9.2.2: the unit for a null array), the array-scalar forms of the logical operators on both
-- sides, ?= and ?/= of bit_vector, to_hstring and to_ostring of bit_vector, and the predefined
-- minimum and maximum of bit_vector, string and integer_vector. Null arrays and widths around the
-- 64-element blocks, both directions.
entity bit_planes is end entity;
architecture a of bit_planes is
  type widths_t is array (natural range <>) of natural;
  constant widths : widths_t := (0, 1, 2, 63, 64, 65, 127, 128, 130);
begin
  process
    variable acc, cases, bad : natural := 0;
    variable seed : natural := 4321;

    impure function rnd(n : positive) return natural is
    begin
      seed := (seed * 75 + 74) mod 65537;
      return seed mod n;
    end function;

    procedure ok(c : boolean; what : string) is
    begin
      cases := cases + 1;
      if not c then
        bad := bad + 1;
        report "mismatch: " & what severity error;
      end if;
    end procedure;

    procedure mix(v : natural) is
    begin
      acc := (acc * 31 + v) mod 1000003;
    end procedure;

    procedure mix(x : bit_vector) is
    begin
      for i in x'range loop mix(bit'pos(x(i))); end loop;
      mix(x'length);
    end procedure;

    procedure mix(x : boolean_vector) is
    begin
      for i in x'range loop mix(boolean'pos(x(i))); end loop;
      mix(x'length);
    end procedure;

    -- op: 0 sll, 1 srl, 2 sla, 3 sra, 4 rol, 5 ror (LRM 9.2.4), left to right.
    function sh(x : bit_vector; op : natural; k : integer) return bit_vector is
      constant n : natural := x'length;
      alias v : bit_vector(0 to n - 1) is x;
      variable r : bit_vector(0 to n - 1);
      variable kk : integer := k;
      variable o : natural := op;
      variable j : integer;
    begin
      if kk < 0 then kk := -kk; o := o + 1 - 2 * (o mod 2); end if;
      for i in 0 to n - 1 loop
        case o is
          when 0 => j := i + kk; if j < n then r(i) := v(j); else r(i) := '0'; end if;
          when 1 => j := i - kk; if j >= 0 then r(i) := v(j); else r(i) := '0'; end if;
          when 2 => j := i + kk; if j < n then r(i) := v(j); else r(i) := v(n - 1); end if;
          when 3 => j := i - kk; if j >= 0 then r(i) := v(j); else r(i) := v(0); end if;
          when 4 => r(i) := v((i + kk) mod n);
          when others => r(i) := v((i - kk) mod n);
        end case;
      end loop;
      return r;
    end function;

    function shb(x : boolean_vector; op : natural; k : integer) return boolean_vector is
      constant n : natural := x'length;
      alias v : boolean_vector(0 to n - 1) is x;
      variable b : bit_vector(0 to n - 1);
      variable r : boolean_vector(0 to n - 1);
    begin
      for i in 0 to n - 1 loop b(i) := bit'val(boolean'pos(v(i))); end loop;
      b := sh(b, op, k);
      for i in 0 to n - 1 loop r(i) := b(i) = '1'; end loop;
      return r;
    end function;

    impure function shift_by(x : bit_vector; op : natural; k : integer) return bit_vector is
    begin
      case op is
        when 0 => return x sll k;
        when 1 => return x srl k;
        when 2 => return x sla k;
        when 3 => return x sra k;
        when 4 => return x rol k;
        when others => return x ror k;
      end case;
    end function;

    impure function shift_byb(x : boolean_vector; op : natural; k : integer) return boolean_vector is
    begin
      case op is
        when 0 => return x sll k;
        when 1 => return x srl k;
        when 2 => return x sla k;
        when 3 => return x sra k;
        when 4 => return x rol k;
        when others => return x ror k;
      end case;
    end function;

    -- The digits of to_hstring (w 4) or to_ostring (w 3): zero-extended on the left.
    function rx(x : bit_vector; w : positive) return string is
      constant nd : natural := (x'length + w - 1) / w;
      variable v : bit_vector(nd * w - 1 downto 0) := (others => '0');
      variable s : string(1 to nd);
      variable d : natural;
      constant digits : string(1 to 16) := "0123456789ABCDEF";
    begin
      v(x'length - 1 downto 0) := x;
      for k in 0 to nd - 1 loop
        d := 0;
        for j in w - 1 downto 0 loop d := d * 2 + bit'pos(v((nd - 1 - k) * w + j)); end loop;
        s(k + 1) := digits(d + 1);
      end loop;
      return s;
    end function;

    -- -1, 0, 1: the order of LRM 9.2.3 by an element loop.
    function lex(x, y : bit_vector) return integer is
      alias a : bit_vector(0 to x'length - 1) is x;
      alias b : bit_vector(0 to y'length - 1) is y;
    begin
      for k in 0 to minimum(x'length, y'length) - 1 loop
        if a(k) /= b(k) then
          if a(k) < b(k) then return -1; else return 1; end if;
        end if;
      end loop;
      if x'length < y'length then return -1; elsif x'length > y'length then return 1; end if;
      return 0;
    end function;

    function lexs(x, y : string) return integer is
      alias a : string(1 to x'length) is x;
      alias b : string(1 to y'length) is y;
    begin
      for k in 1 to minimum(x'length, y'length) loop
        if a(k) /= b(k) then
          if a(k) < b(k) then return -1; else return 1; end if;
        end if;
      end loop;
      if x'length < y'length then return -1; elsif x'length > y'length then return 1; end if;
      return 0;
    end function;

    function lexi(x, y : integer_vector) return integer is
      alias a : integer_vector(0 to x'length - 1) is x;
      alias b : integer_vector(0 to y'length - 1) is y;
    begin
      for k in 0 to minimum(x'length, y'length) - 1 loop
        if a(k) /= b(k) then
          if a(k) < b(k) then return -1; else return 1; end if;
        end if;
      end loop;
      if x'length < y'length then return -1; elsif x'length > y'length then return 1; end if;
      return 0;
    end function;

    type counts_t is array (natural range <>) of integer;

    procedure one_width(n : natural; dir : natural) is
          variable ones, e : natural;
          variable rb, want : bit;
          variable wb : boolean;
          variable x, y : bit_vector(0 to n - 1);
          variable xd : bit_vector(n + 4 downto 5);
          variable xu : bit_vector(3 to n + 2);
          variable bv : boolean_vector(7 to n + 6);
          variable r, q : bit_vector(0 to n - 1);
          variable rbv : boolean_vector(0 to n - 1);
          variable s1, s2 : string(1 to n);
          variable i1, i2 : integer_vector(0 to n - 1);
          constant ks : counts_t := (-n - 1, -3, -1, 0, 1, 3, n - 1, n, n + 2);
        begin
          for i in 0 to n - 1 loop
            x(i) := bit'val(rnd(2));
            bv(7 + i) := rnd(3) = 0;
            s1(i + 1) := character'val(97 + rnd(3));
            i1(i) := rnd(5) - 2;
          end loop;
          y := x;
          s2 := s1;
          i2 := i1;
          if n > 0 then
            e := rnd(n);
            y(e) := not y(e);
            s2(e + 1) := character'val(97 + rnd(3));
            i2(e) := rnd(5) - 2;
          end if;
          xd := x;
          xu := x;
          -- Shifts and rotations.
          for op in 0 to 5 loop
            for ki in ks'range loop
              if op < 2 or op > 3 or ks(ki) >= 0 then
                if dir = 0 then
                  r := shift_by(xd, op, ks(ki));
                else
                  r := shift_by(xu, op, ks(ki));
                end if;
                ok(r = sh(x, op, ks(ki)), "shift " & integer'image(op) & " by "
                   & integer'image(ks(ki)) & " of " & integer'image(n));
                mix(r);
                rbv := shift_byb(bv, op, ks(ki));
                ok(rbv = shb(bv, op, ks(ki)), "boolean shift " & integer'image(op));
                mix(rbv);
              end if;
            end loop;
          end loop;
          -- Reductions.
          ones := 0;
          for i in x'range loop ones := ones + bit'pos(x(i)); end loop;
          ok((and xd) = bit'val(boolean'pos(ones = n)), "and reduce");
          ok((or xu) = bit'val(boolean'pos(ones > 0)), "or reduce");
          ok((xor xd) = bit'val(ones mod 2), "xor reduce");
          ok((nand xu) = bit'val(boolean'pos(ones /= n)), "nand reduce");
          ok((nor xd) = bit'val(boolean'pos(ones = 0)), "nor reduce");
          ok((xnor xu) = bit'val(1 - ones mod 2), "xnor reduce");
          wb := true;
          for i in bv'range loop wb := wb and bv(i); end loop;
          ok((and bv) = wb, "boolean and reduce");
          wb := false;
          for i in bv'range loop wb := wb xor bv(i); end loop;
          ok((xor bv) = wb, "boolean xor reduce");
          mix(ones);
          -- Array-scalar forms.
          for sv in 0 to 1 loop
            rb := bit'val(sv);
            for i in 0 to n - 1 loop q(i) := x(i) and rb; end loop;
            ok((xd and rb) = q, "and as");
            for i in 0 to n - 1 loop q(i) := rb or x(i); end loop;
            ok((rb or xu) = q, "or sa");
            for i in 0 to n - 1 loop q(i) := x(i) xor rb; end loop;
            ok((xd xor rb) = q, "xor as");
            for i in 0 to n - 1 loop q(i) := rb nand x(i); end loop;
            ok((rb nand xu) = q, "nand sa");
            for i in 0 to n - 1 loop q(i) := x(i) nor rb; end loop;
            ok((xd nor rb) = q, "nor as");
            for i in 0 to n - 1 loop q(i) := rb xnor x(i); end loop;
            ok((rb xnor xu) = q, "xnor sa");
            mix(q);
            for i in bv'range loop rbv(i - 7) := bv(i) and (sv = 1); end loop;
            ok((bv and (sv = 1)) = rbv, "boolean and as");
          end loop;
          -- Matching equality of bit_vector.
          want := '1' when x = y else '0';
          ok((x ?= y) = want, "?=");
          ok((xd ?/= y) = not want, "?/=");
          ok((xu ?= x) = '1', "?= self");
          -- to_hstring, to_ostring.
          ok(to_hstring(xd) = rx(x, 4), "to_hstring");
          ok(to_ostring(xu) = rx(x, 3), "to_ostring");
          mix(n);
          -- minimum and maximum of two arrays, and against a prefix.
          if lex(x, y) < 0 then q := x; else q := y; end if;
          ok(minimum(x, y) = q, "minimum bit_vector");
          if lex(x, y) > 0 then q := x; else q := y; end if;
          ok(maximum(xd, y) = q, "maximum bit_vector");
          if n > 0 then
            ok(minimum(x, x(0 to n - 2)) = x(0 to n - 2), "minimum prefix");
            ok(maximum(x(0 to n - 2), xu) = x, "maximum prefix");
          end if;
          if lexs(s1, s2) < 0 then
            ok(minimum(s1, s2) = s1, "minimum string");
          else
            ok(minimum(s1, s2) = s2, "minimum string");
          end if;
          if lexi(i1, i2) > 0 then
            ok(maximum(i1, i2) = i1, "maximum integer_vector");
          else
            ok(maximum(i1, i2) = i2, "maximum integer_vector");
          end if;
          mix(lex(x, y) + 1);
          mix(lexs(s1, s2) + 1);
          mix(lexi(i1, i2) + 1);
    end procedure;
  begin
    for wi in widths'range loop
      for dir in 0 to 1 loop
        one_width(widths(wi), dir);
      end loop;
    end loop;
    report "bit_planes cases=" & integer'image(cases) & " acc=" & integer'image(acc)
      & " bad=" & integer'image(bad);
    assert bad = 0 severity failure;
    wait;
  end process;
end architecture;

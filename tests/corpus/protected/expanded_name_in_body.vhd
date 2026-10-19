-- difftest: top=expanded_name_in_body std=2008 ref=hand
-- Inside a protected body an expanded name with the type as prefix denotes the body's state
-- or a method of the current object, also when a parameter hides the simple name (LRM 8.3,
-- 12.1, #250).
-- ref=hand: one reference simulator rejects `prot_t.get` after another expanded name into the
-- body, another rejects all of them; .expected is the third one's output, the LRM result.
package expanded_name_in_body_p is
  type prot_t is protected
    procedure set (n : integer);
    impure function get return integer;
    impure function twice return integer;
  end protected;
end package;
package body expanded_name_in_body_p is
  type prot_t is protected body
    variable n : integer := 0;
    procedure bump (by : integer) is
    begin
      n := n + by;
    end procedure;
    procedure set (n : integer) is
      procedure nested is
      begin
        prot_t.n := prot_t.n + n;
      end procedure;
    begin
      prot_t.n := n;
      prot_t.bump(1);
      nested;
    end procedure;
    impure function get return integer is
    begin
      return prot_t.n;
    end function;
    impure function twice return integer is
    begin
      return 2 * prot_t.get;
    end function;
  end protected body;
end package body;

use work.expanded_name_in_body_p.all;
entity expanded_name_in_body is
end entity;

architecture a of expanded_name_in_body is
  shared variable a1, a2 : prot_t;
begin
  process
  begin
    a1.set(5);
    a2.set(1);
    report "a1=" & integer'image(a1.get) & " twice=" & integer'image(a1.twice);
    report "a2=" & integer'image(a2.get) & " twice=" & integer'image(a2.twice);
    assert a1.get = 11 and a2.twice = 6 severity failure;
    wait;
  end process;
end architecture;

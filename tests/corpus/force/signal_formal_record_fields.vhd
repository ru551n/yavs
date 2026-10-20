-- difftest: top=signal_formal_record_fields std=2019
-- Scalar signal formals whose actuals are single fields of a record signal, read and waited
-- on in a procedure through a procedure with a mode view parameter (a reference of one part, #429).
entity signal_formal_record_fields is
end entity;

architecture t of signal_formal_record_fields is
  function resolved_max(v : integer_vector) return integer is
    variable m : integer := integer'low;
  begin
    for i in v'range loop
      if v(i) > m then m := v(i); end if;
    end loop;
    return m;
  end function;
  subtype rdy_t is resolved_max integer range 0 to integer'high;
  subtype ack_t is resolved_max integer range -1 to integer'high;
  type rec_t is record
    rdy : rdy_t;
    ack : ack_t;
    addr : bit_vector;
  end record;
  view api of rec_t is
    rdy : out;
    ack : in;
    addr : out;
  end view;
  signal r : rec_t(addr(3 downto 0)) := (0, 0, "0000");

  procedure request (signal rdy : inout rdy_t; signal ack : in ack_t) is
  begin
    rdy <= rdy + 1;
    wait until ack = rdy;
    report "ack " & integer'image(ack) & " rdy " & integer'image(rdy);
  end procedure;
  procedure outer (signal tr : view api of rec_t) is
  begin
    request(rdy => tr.rdy, ack => tr.ack);
  end procedure;
begin
  p : process
  begin
    outer(r);
    outer(r);
    wait;
  end process;

  q : process
  begin
    wait on r.rdy;
    r.ack <= r.rdy after 1 ns;
  end process;
end architecture;

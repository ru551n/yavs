-- A record resolved as a whole with three drivers that all assign at once: the resolution
-- function is called once for the update, with the three records (LRM 14.7.3.2).
entity top is end;
architecture a of top is
  type rec is record a : integer; b : bit; end record;
  type rec_v is array (natural range <>) of rec;
  function res_rec (v : rec_v) return rec is
    variable r : rec := (0, '0');
  begin
    for i in v'range loop r.a := r.a + v(i).a; r.b := r.b or v(i).b; end loop;
    if r.a /= 0 then
      report "RES call with " & integer'image(v'length) & " sources: " & integer'image(r.a);
    end if;
    return r;
  end;
  subtype rrec is res_rec rec;
  signal rr : rrec := (0, '0');
begin
  g : for k in 1 to 3 generate
    process begin
      rr <= (k, '1') after 1 ns;
      wait;
    end process;
  end generate;
  process (rr) begin
    if rr.a /= 0 then report "RES rr.a = " & integer'image(rr.a); end if;
  end process;
end;

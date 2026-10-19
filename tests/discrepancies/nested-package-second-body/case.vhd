-- Two bodies of one package declared in an architecture (LRM 4.8: a package has one body).
entity top is end;
architecture a of top is
  package p is
  end package;
  package body p is
  end package body;
  package body p is
  end package body;
begin
end;

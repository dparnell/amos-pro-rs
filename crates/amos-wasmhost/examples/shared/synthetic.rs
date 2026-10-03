// Included by the `suite` example and `amos-build`'s `bundle_synthetic`.

/// Synthetic workloads (screen, text, graphics keywords, maths, arrays).
pub const SYNTHETIC: &[(&str, &str)] = &[
    (
        "plot/draw",
        "Screen Open 0,320,200,16,Lowres\nDo\nFor I=0 To 199 : Ink I mod 16 : Plot I,I/2 : Draw I,0 To 319-I,199 : Next\nLoop",
    ),
    ("locate/print", "Do\nFor I=0 To 20 : Locate 0,I : Print \"Line\";I;\" \";A : Inc A : Next\nLoop"),
    (
        "bar/box/circle",
        "Screen Open 0,320,200,16,Lowres\nDo\nFor I=0 To 60 : Ink I mod 16 : Bar I,I To I+40,I+30 : Box I,I To I+50,I+40 : Circle 160,100,I+1 : Next\nLoop",
    ),
    (
        "maths + arrays",
        "Dim A#(1000),B(1000)\nDo\nFor I=0 To 1000 : A#(I)=Sin(I)*Cos(I/2)+Sqr(I) : B(I)=(B(I)+I*3) mod 977 : Next : Inc F\nLoop",
    ),
    (
        "strings",
        "Do\nA$=\"\" : For I=1 To 100 : A$=A$+Chr$(65+I mod 26) : Next : B$=Upper$(Lower$(A$)) : C=Instr(B$,\"XYZ\") : Inc F\nLoop",
    ),
    (
        "procedures",
        "Do\nFor I=1 To 100 : P[I] : S=S+Param : Next\nLoop\nProcedure P[N]\nIf N<2 Then Pop Proc[N]\nEnd Proc[N*2+1]",
    ),
    ("for/next integer", "Do\nFor I=1 To 1000 : A=A+I : Next I\nLoop"),
    ("for/next float", "Do\nX#=0 : For F#=1 To 1000 : X#=X#+F# : Next F#\nLoop"),
    ("for/next nested", "Do\nFor I=1 To 100 : For J=1 To 5 : Inc N : Next J : Next I\nLoop"),
    ("for/next with keyword", "Do\nFor I=0 To 199 : Plot I,I/2 : Next I\nLoop"),
    ("busy wait (Scin, X/Y Mouse, Mouse Key)", "Do : N=Scin(X Mouse,Y Mouse) : K=Mouse Key : Loop"),
    ("inkey$ / joy / timer", "Do : A$=Inkey$ : J=Joy(1) : T=Timer : K=Key State(69) : Loop"),
    (
        "bit operations on variables",
        "V=1\nDo\nFor I=1 To 1000 : Bset I and 31,V : Ror.w 3,V : Bclr 5,V : Rol.b 1,V : Next\nLoop",
    ),
    (
        "game main loop",
        "Screen Open 0,320,200,16,Lowres : Curs Off\nDim X(50),Y(50),DX(50),DY(50)\nFor I=0 To 50 : X(I)=Rnd(300) : Y(I)=Rnd(180) : DX(I)=1+Rnd(2) : DY(I)=1+Rnd(2) : Next\nDo\nCls 0\nFor I=0 To 50\nX(I)=X(I)+DX(I) : Y(I)=Y(I)+DY(I)\nIf X(I)<0 or X(I)>310 Then DX(I)=-DX(I)\nIf Y(I)<0 or Y(I)>190 Then DY(I)=-DY(I)\nInk 1+I mod 15 : Bar X(I),Y(I) To X(I)+8,Y(I)+8\nNext\nX=X Mouse : Y=Y Mouse : K=Inkey$<>\"\"\nWait Vbl\nLoop",
    ),
];

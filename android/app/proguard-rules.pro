# BouncyCastle: keep the lightweight primitives we use.
-keep class org.bouncycastle.crypto.** { *; }
-keep class org.bouncycastle.math.ec.rfc7748.** { *; }
-dontwarn org.bouncycastle.**
-dontwarn javax.naming.**

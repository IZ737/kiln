-- 0001 init
-- Written by `kiln db diff` for mysql. Read it before applying it.

create table users (
    id bigint auto_increment primary key,
    email text not null,
    name text not null,
    password text not null,
    role text not null
);

create table posts (
    id bigint auto_increment primary key,
    user_id bigint not null,
    title text not null,
    body text not null,
    published boolean not null,
    foreign key (user_id) references users(id)
);

create table comments (
    id bigint auto_increment primary key,
    post_id bigint not null,
    user_id bigint not null,
    body text not null,
    foreign key (post_id) references posts(id),
    foreign key (user_id) references users(id)
);


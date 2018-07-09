package edu.rpi.aris.assign.message;

import edu.rpi.aris.assign.NetUtil;
import edu.rpi.aris.assign.User;

import java.sql.Connection;
import java.sql.PreparedStatement;
import java.sql.ResultSet;
import java.sql.SQLException;

public class ClassCreateMsg extends Message {

    private final String name;
    private int cid;

    public ClassCreateMsg(String name) {
        this.name = name;
    }

    // DO NOT REMOVE!! Default constructor is required for gson deserialization
    private ClassCreateMsg() {
        name = null;
    }

    public int getClassId() {
        return cid;
    }

    @Override
    public ErrorType processMessage(Connection connection, User user) throws SQLException {
        if (!user.userType.equals(NetUtil.USER_INSTRUCTOR))
            return ErrorType.UNAUTHORIZED;
        try (PreparedStatement insertClass = connection.prepareStatement("INSERT INTO class (name) VALUES(?);");
             PreparedStatement selectClassId = connection.prepareStatement("SELECT id FROM class ORDER BY id DESC LIMIT 1;");
             PreparedStatement insertUserClass = connection.prepareStatement("INSERT INTO user_class VALUES(?, ?);")) {
            insertClass.setString(1, name);
            insertClass.executeUpdate();
            try (ResultSet rs = selectClassId.executeQuery()) {
                if (!rs.next())
                    return ErrorType.SQL_ERR;
                cid = rs.getInt(1);
            }
            insertUserClass.setInt(1, user.uid);
            insertUserClass.setInt(2, cid);
            insertUserClass.executeUpdate();
        }
        return null;
    }

    @Override
    public MessageType getMessageType() {
        return MessageType.CREATE_CLASS;
    }

    @Override
    public boolean checkValid() {
        return name != null;
    }
}
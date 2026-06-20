import React from 'react';
import { NavigationContainer } from '@react-navigation/native';
import { createNativeStackNavigator } from '@react-navigation/native-stack';
import RegisterScreen from './screens/RegisterScreen';
import VoteScreen from './screens/VoteScreen';

export type RootStackParamList = {
  Register: undefined;
  Vote: undefined;
};

const Stack = createNativeStackNavigator<RootStackParamList>();

export default function App() {
  return (
    <NavigationContainer>
      <Stack.Navigator initialRouteName="Register">
        <Stack.Screen name="Register" component={RegisterScreen} options={{ title: 'Get Started' }} />
        <Stack.Screen name="Vote" component={VoteScreen} options={{ title: 'Proposals' }} />
      </Stack.Navigator>
    </NavigationContainer>
  );
}
